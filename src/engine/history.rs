//! Facilities for tracking and persisting the shell's command history.

use std::{
    collections::{HashMap, VecDeque},
    ffi::OsString,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::engine::error;

/// Represents a unique identifier for a history item.
type ItemId = i64;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn parse_timestamp_boundary(line: &str) -> Option<ItemTimestamp> {
    let seconds_since_epoch = line.strip_prefix('#')?.parse::<u64>().ok()?;
    Some(ItemTimestamp::from_epoch(seconds_since_epoch))
}

fn push_imported_record(
    imported: &mut VecDeque<(String, Option<ItemTimestamp>)>,
    command_line: String,
    timestamp: Option<ItemTimestamp>,
    max_items: Option<usize>,
) {
    if max_items == Some(0) {
        return;
    }

    imported.push_back((command_line, timestamp));
    if let Some(max_items) = max_items
        && imported.len() > max_items
    {
        imported.pop_front();
    }
}

fn create_history_temp_file(target: &Path) -> std::io::Result<(std::fs::File, PathBuf)> {
    let file_name = target.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "history path has no file name",
        )
    })?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));

    for _ in 0..128 {
        let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temp_name = OsString::from(".");
        temp_name.push(file_name);
        temp_name.push(format!(".{}.{sequence}.tmp", std::process::id()));
        let temp_path = parent.join(temp_name);

        match std::fs::File::options()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(temp_file) => {
                if let Ok(metadata) = std::fs::metadata(target)
                    && let Err(err) = temp_file.set_permissions(metadata.permissions())
                {
                    drop(temp_file);
                    let _ = std::fs::remove_file(&temp_path);
                    return Err(err);
                }
                return Ok((temp_file, temp_path));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not create a unique history temporary file",
    ))
}

/// Interface for querying and manipulating the shell's recorded history of commands.
#[derive(Clone, Default)]
pub struct History {
    items: Vec<ItemId>,
    id_map: HashMap<ItemId, Item>,
    next_id: ItemId,
    revision: u64,
    non_append_revision: u64,
}

impl History {
    /// Constructs a new `History` instance, with its contents initialized from the given readable
    /// stream. If errors are encountered reading lines from the stream, unreadable lines will
    /// be skipped but the call will still return successfully, with a warning logged. An error
    /// result will be returned only if an internal error occurs updating the history.
    ///
    /// # Arguments
    ///
    /// * `reader` - The readable stream to import history from.
    pub fn import(reader: impl Read) -> Result<Self, error::Error> {
        Self::import_with_limit(reader, None)
    }

    /// 从流中导入 history, 并仅保留最后 `max_items` 条记录.
    pub fn import_with_limit(
        reader: impl Read,
        max_items: Option<usize>,
    ) -> Result<Self, error::Error> {
        let buf_reader = std::io::BufReader::new(reader);
        let mut imported = VecDeque::new();
        let mut extended_record: Option<(String, ItemTimestamp, bool)> = None;

        for line_result in buf_reader.lines() {
            let line = match line_result {
                Ok(line) => line,
                // 无法解码的行可能包含无效 UTF-8, 跳过后继续尽力导入.
                Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                    log::warn!("unreadable history line; {err}");
                    continue;
                }
                // 其他 I/O 错误可能持续发生, 立即返回以避免失败循环.
                Err(err) => return Err(err.into()),
            };

            if let Some(timestamp) = parse_timestamp_boundary(&line) {
                if let Some((command_line, timestamp, has_line)) = extended_record.take()
                    && has_line
                {
                    push_imported_record(&mut imported, command_line, Some(timestamp), max_items);
                }
                extended_record = Some((String::new(), timestamp, false));
                continue;
            }

            if let Some((command_line, _, has_line)) = &mut extended_record {
                if *has_line {
                    command_line.push('\n');
                }
                command_line.push_str(&line);
                *has_line = true;
            } else if !line.starts_with('#') {
                push_imported_record(&mut imported, line, None, max_items);
            }
        }

        if let Some((command_line, timestamp, has_line)) = extended_record
            && has_line
        {
            push_imported_record(&mut imported, command_line, Some(timestamp), max_items);
        }

        let mut history = Self::default();
        for (command_line, timestamp) in imported {
            history.add(Item {
                id: 0,
                command_line,
                timestamp,
                dirty: false,
            })?;
        }
        Ok(history)
    }

    /// Tries to retrieve a history item by its unique identifier. Returns `None` if no item is
    /// found.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier of the history item to retrieve.
    pub fn get_by_id(&self, id: ItemId) -> Result<Option<&Item>, error::Error> {
        Ok(self.id_map.get(&id))
    }

    /// Replaces the history item with the given ID with a new item. Returns an error if the item
    /// cannot be updated.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier of the history item to update.
    /// * `item` - The new history item to replace the old one.
    pub fn update_by_id(&mut self, id: ItemId, item: Item) -> Result<(), error::Error> {
        let existing_item = self
            .id_map
            .get_mut(&id)
            .ok_or(error::ErrorKind::HistoryItemNotFound)?;
        *existing_item = item;
        self.note_non_append_change();
        Ok(())
    }

    /// Removes the nth item from the history. Returns the removed item, or `None` if no such item
    /// exists (i.e., because it was out of range).
    pub fn remove_nth_item(&mut self, n: usize) -> bool {
        if n >= self.items.len() {
            return false;
        }

        let id = self.items.remove(n);
        self.id_map.remove(&id);
        self.note_non_append_change();
        true
    }

    /// Adds a new history item. Returns the unique identifier of the newly added item.
    ///
    /// # Arguments
    ///
    /// * `item` - The history item to add.
    pub fn add(&mut self, mut item: Item) -> Result<ItemId, error::Error> {
        let id = self.next_id;

        item.id = id;
        self.next_id += 1;

        self.items.push(item.id);
        self.id_map.insert(item.id, item);
        self.revision = self.revision.wrapping_add(1);

        Ok(id)
    }

    /// Deletes a history item by its unique identifier. Returns an error if the item cannot be
    /// deleted.
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier of the history item to delete.
    pub fn delete_item_by_id(&mut self, id: ItemId) -> Result<(), error::Error> {
        self.id_map.remove(&id);
        self.items.retain(|item_id| *item_id != id);
        self.note_non_append_change();

        Ok(())
    }

    /// Clears all history items.
    pub fn clear(&mut self) -> Result<(), error::Error> {
        self.id_map.clear();
        self.items.clear();
        self.note_non_append_change();
        Ok(())
    }

    /// Flushes the history to backing storage (if relevant).
    ///
    /// # Arguments
    ///
    /// * `history_file_path` - The path to the history file.
    /// * `append` - Whether to append to the file or overwrite it.
    /// * `unsaved_items_only` - Whether to only write unsaved items; if true, any items will be
    ///   marked as "saved" once saved.
    /// * `write_timestamps` - Whether to write timestamps for each command line.
    pub fn flush(
        &mut self,
        history_file_path: impl AsRef<Path>,
        append: bool,
        unsaved_items_only: bool,
        write_timestamps: bool,
    ) -> Result<(), error::Error> {
        let history_file_path = history_file_path.as_ref();
        if append {
            let mut file = std::fs::File::options()
                .create(true)
                .append(true)
                .open(history_file_path)?;
            self.flush_to_writer(&mut file, unsaved_items_only, write_timestamps)
        } else {
            self.flush_overwrite(history_file_path, unsaved_items_only, write_timestamps)
        }
    }

    fn flush_to_writer(
        &mut self,
        writer: &mut impl Write,
        unsaved_items_only: bool,
        write_timestamps: bool,
    ) -> Result<(), error::Error> {
        let saved_item_ids = self.write_records(writer, unsaved_items_only, write_timestamps)?;
        writer.flush()?;
        self.mark_items_saved(&saved_item_ids);
        Ok(())
    }

    fn flush_overwrite(
        &mut self,
        history_file_path: &Path,
        unsaved_items_only: bool,
        write_timestamps: bool,
    ) -> Result<(), error::Error> {
        self.flush_overwrite_with(
            history_file_path,
            unsaved_items_only,
            write_timestamps,
            |temp_path, target_path| std::fs::rename(temp_path, target_path),
        )
    }

    fn flush_overwrite_with(
        &mut self,
        history_file_path: &Path,
        unsaved_items_only: bool,
        write_timestamps: bool,
        replace: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<(), error::Error> {
        let (mut temp_file, temp_path) = create_history_temp_file(history_file_path)?;
        let write_result = (|| -> std::io::Result<Vec<ItemId>> {
            let saved_item_ids =
                self.write_records(&mut temp_file, unsaved_items_only, write_timestamps)?;
            temp_file.flush()?;
            temp_file.sync_all()?;
            Ok(saved_item_ids)
        })();
        drop(temp_file);

        let saved_item_ids = match write_result {
            Ok(saved_item_ids) => saved_item_ids,
            Err(err) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(err.into());
            }
        };

        if let Err(err) = replace(&temp_path, history_file_path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(err.into());
        }

        self.mark_items_saved(&saved_item_ids);
        Ok(())
    }

    fn write_records(
        &self,
        writer: &mut impl Write,
        unsaved_items_only: bool,
        write_timestamps: bool,
    ) -> std::io::Result<Vec<ItemId>> {
        let write_boundaries = write_timestamps
            || self.items.iter().any(|item_id| {
                self.id_map.get(item_id).is_some_and(|item| {
                    (!unsaved_items_only || item.dirty) && item.command_line.contains('\n')
                })
            });
        let mut saved_item_ids = Vec::new();

        for item_id in &self.items {
            let Some(item) = self.id_map.get(item_id) else {
                continue;
            };
            if unsaved_items_only && !item.dirty {
                continue;
            }

            let mut record = Vec::with_capacity(item.command_line.len() + 32);
            if write_boundaries {
                let timestamp = item.timestamp.map_or(0, |value| value.to_epoch_secs());
                record.extend_from_slice(format!("#{timestamp}\n").as_bytes());
            }
            record.extend_from_slice(item.command_line.as_bytes());
            record.push(b'\n');

            // 每条记录只提交一个 buffer, 避免并发 append 时边界与命令分离.
            writer.write_all(&record)?;
            if item.dirty {
                saved_item_ids.push(*item_id);
            }
        }

        Ok(saved_item_ids)
    }

    fn mark_items_saved(&mut self, item_ids: &[ItemId]) {
        for item_id in item_ids {
            if let Some(item) = self.id_map.get_mut(item_id) {
                item.dirty = false;
            }
        }
    }

    /// Searches through history using the given query.
    ///
    /// # Arguments
    ///
    /// * `query` - The query to use.
    pub fn search(&self, query: Query) -> Result<impl Iterator<Item = &self::Item>, error::Error> {
        Ok(Search::new(self, query))
    }

    /// Returns an iterator over the history items.
    pub fn iter(&self) -> impl Iterator<Item = &self::Item> {
        Search::all(self)
    }

    /// Retrieves the nth history item, if it exists. Returns `None` if no such item exists.
    /// Indexing is zero-based, with an index of 0 referencing the oldest item in the history.
    ///
    /// # Arguments
    ///
    /// * `index` - The index of the history item to retrieve.
    pub fn get(&self, index: usize) -> Option<&Item> {
        if let Some(id) = self.items.get(index) {
            self.id_map.get(id)
        } else {
            None
        }
    }

    /// Returns the number of items in the history.
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// 返回每次内容变化都会更新的版本号.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// 返回仅在非追加变化时更新的版本号.
    pub const fn non_append_revision(&self) -> u64 {
        self.non_append_revision
    }

    /// 仅保留最新的 `max_items` 条记录.
    pub fn truncate_to_max_items(&mut self, max_items: usize) {
        if self.items.len() <= max_items {
            return;
        }

        let remove_count = self.items.len() - max_items;
        for id in self.items.drain(..remove_count) {
            self.id_map.remove(&id);
        }
        self.note_non_append_change();
    }

    fn note_non_append_change(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.non_append_revision = self.non_append_revision.wrapping_add(1);
    }
}

/// Represents a timestamp for a history item.
pub type ItemTimestamp = nanotime::NanoTime;

/// Represents an item in the history.
#[derive(Clone, Default)]
pub struct Item {
    /// The unique identifier of the history item.
    pub id: ItemId,
    /// The actual command line.
    pub command_line: String,
    /// The timestamp when the command was started.
    pub timestamp: Option<ItemTimestamp>,
    /// Whether or not the item is dirty, i.e., has not yet been written to backing storage.
    pub dirty: bool,
}

impl Item {
    /// Constructs a new `Item` with the given command line.
    ///
    /// # Arguments
    ///
    /// * `command_line` - The command line of the item.
    pub fn new(command_line: impl Into<String>) -> Self {
        Self {
            id: 0, // NOTE: ID will be assigned when added to the history.
            command_line: command_line.into(),
            timestamp: Some(nanotime::NanoTime::now_utc()),
            dirty: true,
        }
    }
}

/// Encapsulates query parameters for searching through history.
#[derive(Default)]
pub struct Query {
    /// Whether to search forward or backward
    pub direction: Direction,
    /// Optionally, clamp results to items with a timestamp strictly after this.
    pub not_at_or_before_time: Option<ItemTimestamp>,
    /// Optionally, clamp results to items with a timestamp strictly before this.
    pub not_at_or_after_time: Option<ItemTimestamp>,
    /// Optionally, clamp results to items with an ID equal strictly after this.
    pub not_at_or_before_id: Option<ItemId>,
    /// Optionally, clamp results to items with an ID equal strictly before this.
    pub not_at_or_after_id: Option<ItemId>,
    /// Optionally, maximum number of items to retrieve
    pub max_items: Option<i64>,
    /// Optionally, a string-based filter on command line.
    pub command_line_filter: Option<CommandLineFilter>,
}

impl Query {
    /// Checks if the query includes the given item.
    ///
    /// # Arguments
    ///
    /// * `item` - The item to check.
    pub fn includes(&self, item: &Item) -> bool {
        // Filter based on not_at_or_before_time.
        if let Some(not_at_or_before_time) = &self.not_at_or_before_time
            && item
                .timestamp
                .is_some_and(|ts| ts <= *not_at_or_before_time)
        {
            return false;
        }

        // Filter based on not_at_or_after_time
        if let Some(not_at_or_after_time) = &self.not_at_or_after_time
            && item.timestamp.is_some_and(|ts| ts >= *not_at_or_after_time)
        {
            return false;
        }

        // Filter based on not_at_or_before_id
        if self
            .not_at_or_before_id
            .is_some_and(|query_id| item.id <= query_id)
        {
            return false;
        }

        // Filter based on not_at_or_after_id
        if self
            .not_at_or_after_id
            .is_some_and(|query_id| item.id >= query_id)
        {
            return false;
        }

        // Filter based on command_line_filter
        if let Some(command_line_filter) = &self.command_line_filter {
            match command_line_filter {
                CommandLineFilter::Prefix(prefix) => {
                    if !item.command_line.starts_with(prefix) {
                        return false;
                    }
                }
                CommandLineFilter::Suffix(suffix) => {
                    if !item.command_line.ends_with(suffix) {
                        return false;
                    }
                }
                CommandLineFilter::Contains(contains) => {
                    if !item.command_line.contains(contains) {
                        return false;
                    }
                }
                CommandLineFilter::Exact(exact) => {
                    if item.command_line != *exact {
                        return false;
                    }
                }
            }
        }

        true
    }
}

/// Represents the direction of a search operation.
#[derive(Default)]
pub enum Direction {
    /// Search forward from the oldest part of history.
    #[default]
    Forward,
    /// Search backward from the youngest part of history.
    Backward,
}

/// Filter criteria for command lines.
pub enum CommandLineFilter {
    /// The command line must start with this string.
    Prefix(String),
    /// The command line must end with this string.
    Suffix(String),
    /// The command line must contain this string.
    Contains(String),
    /// The command line must match this string exactly.
    Exact(String),
}

/// Represents a search operation.
pub struct Search<'a> {
    /// The history to search through.
    history: &'a History,
    /// The query to apply.
    query: Query,
    /// The next index in `items`.
    next_index: Option<usize>,
    /// Count of items returned so far.
    count: usize,
}

impl<'a> Search<'a> {
    /// Constructs a new search against the provided history, querying *all* items.
    ///
    /// # Arguments
    ///
    /// * `history` - The history to search through.
    pub fn all(history: &'a History) -> Self {
        Self::new(history, Query::default())
    }

    /// Constructs a new search against the provided history, using the given query.
    ///
    /// # Arguments
    ///
    /// * `history` - The history to search through.
    /// * `query` - The query to use.
    pub fn new(history: &'a History, query: Query) -> Self {
        let next_index = match query.direction {
            Direction::Forward => Some(0),
            Direction::Backward => {
                if history.items.is_empty() {
                    None
                } else {
                    Some(history.items.len() - 1)
                }
            }
        };

        Self {
            history,
            query,
            next_index,
            count: 0,
        }
    }

    const fn increment_next_index(&mut self) {
        if let Some(index) = self.next_index {
            self.next_index = match self.query.direction {
                Direction::Forward => Some(index + 1),
                Direction::Backward => {
                    if index == 0 {
                        None
                    } else {
                        Some(index - 1)
                    }
                }
            }
        }
    }
}

impl<'a> Iterator for Search<'a> {
    type Item = &'a Item;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(index) = self.next_index {
                // Make sure we haven't hit the end of the history.
                if index >= self.history.items.len() {
                    return None;
                }

                let id = self.history.items[index];
                self.increment_next_index();

                if let Some(item) = self.history.id_map.get(&id) {
                    // Filter based on max_items. Once we hit the limit,
                    // we stop searching.
                    #[expect(clippy::cast_possible_truncation)]
                    #[expect(clippy::cast_sign_loss)]
                    if self
                        .query
                        .max_items
                        .is_some_and(|max_items| self.count >= max_items as usize)
                    {
                        return None;
                    }

                    // Check other filters. If they don't match, then we
                    // skip but keep searching.
                    if self.query.includes(item) {
                        self.count += 1;
                        return Some(item);
                    }
                }
            } else {
                return None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_limit_keeps_newest_items() {
        let history = History::import_with_limit("one\ntwo\nthree\n".as_bytes(), Some(2)).unwrap();
        let commands = history
            .iter()
            .map(|item| item.command_line.as_str())
            .collect::<Vec<_>>();
        assert_eq!(commands, ["two", "three"]);
    }

    #[test]
    fn append_and_non_append_revisions_are_distinct() {
        let mut history = History::default();
        history.add(Item::new("one")).unwrap();
        let append_revision = history.revision();
        assert_eq!(history.non_append_revision(), 0);

        history.remove_nth_item(0);
        assert!(history.revision() > append_revision);
        assert_eq!(history.non_append_revision(), 1);
    }

    #[test]
    fn truncation_keeps_newest_items() {
        let mut history = History::default();
        for command in ["one", "two", "three"] {
            history.add(Item::new(command)).unwrap();
        }

        history.truncate_to_max_items(2);
        let commands = history
            .iter()
            .map(|item| item.command_line.as_str())
            .collect::<Vec<_>>();
        assert_eq!(commands, ["two", "three"]);
    }

    #[derive(Default)]
    struct RecordingWriter {
        payload: Vec<u8>,
        write_count: usize,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.write_count += 1;
            self.payload.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("写入失败"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FailingFlushWriter {
        payload: Vec<u8>,
    }

    impl Write for FailingFlushWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.payload.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("刷新失败"))
        }
    }

    #[test]
    fn append_writes_each_record_as_one_buffer() {
        let mut history = History::default();
        history
            .add(Item {
                timestamp: Some(ItemTimestamp::from_epoch(1)),
                ..Item::new("one")
            })
            .unwrap();
        history
            .add(Item {
                timestamp: Some(ItemTimestamp::from_epoch(2)),
                ..Item::new("two")
            })
            .unwrap();
        let mut writer = RecordingWriter::default();

        history.flush_to_writer(&mut writer, true, true).unwrap();

        assert_eq!(writer.write_count, 2);
        assert_eq!(writer.payload, b"#1\none\n#2\ntwo\n");
        assert!(history.iter().all(|item| !item.dirty));
    }

    #[test]
    fn multiline_records_roundtrip_with_extended_history_boundaries() {
        let mut history = History::default();
        for (timestamp, command) in [
            (1, "one"),
            (2, "if true; then\n  echo two\nfi"),
            (3, "three"),
        ] {
            history
                .add(Item {
                    timestamp: Some(ItemTimestamp::from_epoch(timestamp)),
                    ..Item::new(command)
                })
                .unwrap();
        }
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("history");
        std::fs::write(&path, "stale\n").unwrap();

        history.flush(&path, false, false, false).unwrap();
        let payload = std::fs::read(&path).unwrap();
        let imported = History::import(std::fs::File::open(path).unwrap()).unwrap();
        let commands = imported
            .iter()
            .map(|item| item.command_line.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            payload,
            b"#1\none\n#2\nif true; then\n  echo two\nfi\n#3\nthree\n"
        );
        assert_eq!(commands, ["one", "if true; then\n  echo two\nfi", "three"]);
    }

    #[test]
    fn import_limit_counts_extended_records_instead_of_physical_lines() {
        let history = History::import_with_limit(
            b"#1\none\ncontinued\n#2\ntwo\n#3\nthree\n".as_slice(),
            Some(2),
        )
        .unwrap();
        let commands = history
            .iter()
            .map(|item| item.command_line.as_str())
            .collect::<Vec<_>>();

        assert_eq!(commands, ["two", "three"]);
    }

    #[test]
    fn failed_write_preserves_dirty_markers() {
        let mut history = History::default();
        history.add(Item::new("one")).unwrap();

        assert!(
            history
                .flush_to_writer(&mut FailingWriter, true, false)
                .is_err()
        );
        assert!(history.get(0).unwrap().dirty);
    }

    #[test]
    fn failed_flush_preserves_dirty_markers() {
        let mut history = History::default();
        history.add(Item::new("one")).unwrap();
        let mut writer = FailingFlushWriter::default();

        assert!(history.flush_to_writer(&mut writer, true, false).is_err());
        assert_eq!(writer.payload, b"one\n");
        assert!(history.get(0).unwrap().dirty);
    }

    #[test]
    fn failed_atomic_replace_preserves_target_and_dirty_markers() {
        let scratch = tempfile::tempdir().unwrap();
        let target = scratch.path().join("history");
        std::fs::write(&target, "original\n").unwrap();
        let mut history = History::default();
        history.add(Item::new("replacement")).unwrap();
        let mut observed_temp_path = None;

        let result =
            history.flush_overwrite_with(&target, false, false, |temp_path, target_path| {
                assert_eq!(temp_path.parent(), target_path.parent());
                assert_eq!(std::fs::read_to_string(temp_path).unwrap(), "replacement\n");
                observed_temp_path = Some(temp_path.to_owned());
                Err(std::io::Error::other("替换失败"))
            });

        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original\n");
        assert!(history.get(0).unwrap().dirty);
        assert!(!observed_temp_path.unwrap().exists());
    }
}
