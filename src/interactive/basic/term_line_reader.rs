//
// This module is intentionally limited, and does not have all the bells and whistles. We want
// enough here that we can use it in the basic shell for (p)expect/pty-style testing of
// completion, and without using VT100-style escape sequences for cursor movement and display.
//

use crate::interactive::term::{self, KeyCode};
use crate::interactive::{ReadResult, ShellError};
use std::io::Write;

const BACKSPACE: char = 8u8 as char;
const MAX_COMPLETION_COLUMNS: usize = 4;

pub(crate) struct TermLineReader {
    _console_mode: ConsoleModeGuard,
}

struct ConsoleModeGuard {
    original_mode: u32,
}

impl ConsoleModeGuard {
    fn new() -> Result<Self, ShellError> {
        let original_mode = term::enable_raw_mode()?;
        Ok(Self { original_mode })
    }
}

impl Drop for ConsoleModeGuard {
    fn drop(&mut self) {
        let _ = term::restore_console_mode(self.original_mode);
    }
}

impl TermLineReader {
    pub fn new() -> Result<Self, ShellError> {
        Ok(Self {
            _console_mode: ConsoleModeGuard::new()?,
        })
    }
}

impl super::LineReader for TermLineReader {
    fn read_line(
        &self,
        prompt: Option<&str>,
        history_entries: &[String],
        mut completion_handler: impl FnMut(
            &str,
            usize,
        )
            -> Result<crate::engine::completion::Completions, ShellError>,
    ) -> Result<ReadResult, ShellError> {
        let mut state = ReadLineState::new(prompt, history_entries);
        state.display_prompt()?;

        loop {
            let key_event = term::read_key_event()?;
            if let Some(result) = state.on_key(key_event, &mut completion_handler)? {
                return Ok(result);
            }
        }
    }
}

struct ReadLineState<'a> {
    // Current line of input
    line: String,
    // Current position of cursor, expressed as a byte offset from the
    // start of `line`. We maintain the invariant that it will always
    // be at a clean character boundary.
    cursor: usize,
    // Current prompt to use.
    prompt: Option<&'a str>,
    input_origin: Option<term::CursorPosition>,
    history_entries: &'a [String],
    history_position: Option<usize>,
    history_draft: Option<String>,
    completion_menu: Option<CompletionMenu>,
}

struct CompletionMenu {
    completions: crate::engine::completion::Completions,
    selected: usize,
    rows: usize,
    columns: usize,
    rendered_lines: usize,
    cursor_visibility: Option<term::CursorVisibilityGuard>,
}

#[derive(Clone, Copy)]
enum CompletionDirection {
    Up,
    Down,
    Left,
    Right,
}

impl<'a> ReadLineState<'a> {
    const fn new(prompt: Option<&'a str>, history_entries: &'a [String]) -> Self {
        Self {
            line: String::new(),
            cursor: 0,
            prompt,
            input_origin: None,
            history_entries,
            history_position: None,
            history_draft: None,
            completion_menu: None,
        }
    }

    pub fn display_prompt(&mut self) -> Result<(), ShellError> {
        if let Some(prompt) = self.prompt {
            eprint!("{prompt}");
            std::io::stderr().flush()?;
        }

        self.input_origin = term::get_cursor_position().ok();
        Ok(())
    }

    fn on_key(
        &mut self,
        event: term::KeyEvent,
        mut completion_handler: impl FnMut(
            &str,
            usize,
        )
            -> Result<crate::engine::completion::Completions, ShellError>,
    ) -> Result<Option<ReadResult>, ShellError> {
        match (&event.modifiers, &event.code) {
            (_, KeyCode::Enter) if self.completion_menu.is_some() => {
                self.accept_completion_selection()?;
            }
            (_, KeyCode::Enter) => {
                self.clear_completion_menu()?;
                if !self.line.trim().is_empty() {
                    Self::display_newline()?;
                }
                self.line.push('\n');
                let line = std::mem::take(&mut self.line);
                return Ok(Some(ReadResult::Input(line)));
            }
            (mods, KeyCode::Char('j')) if mods.ctrl => {
                self.clear_completion_menu()?;
                if !self.line.trim().is_empty() {
                    Self::display_newline()?;
                }
                self.line.push('\n');
                let line = std::mem::take(&mut self.line);
                return Ok(Some(ReadResult::Input(line)));
            }
            (mods, KeyCode::Char(c)) if !mods.ctrl && !mods.alt => {
                self.clear_completion_menu()?;
                self.on_char(*c)?;
            }
            (mods, KeyCode::Char('c')) if mods.ctrl => {
                self.clear_completion_menu()?;
                eprintln!("^C");
                return Ok(Some(ReadResult::Interrupted));
            }
            (mods, KeyCode::Char('d')) if mods.ctrl && self.line.is_empty() => {
                self.clear_completion_menu()?;
                Self::display_newline()?;
                return Ok(Some(ReadResult::Eof));
            }
            (mods, KeyCode::Char('l')) if mods.ctrl => {
                self.clear_completion_menu()?;
                self.clear_screen()?;
            }
            (_, KeyCode::Backspace) => {
                self.clear_completion_menu()?;
                self.backspace()?;
            }
            (_, KeyCode::Up) if self.completion_menu.is_some() => {
                self.move_completion_selection(CompletionDirection::Up)?;
            }
            (_, KeyCode::Down) if self.completion_menu.is_some() => {
                self.move_completion_selection(CompletionDirection::Down)?;
            }
            (_, KeyCode::Left) if self.completion_menu.is_some() => {
                self.move_completion_selection(CompletionDirection::Left)?;
            }
            (_, KeyCode::Right) if self.completion_menu.is_some() => {
                self.move_completion_selection(CompletionDirection::Right)?;
            }
            (_, KeyCode::Up) => {
                self.move_history_previous()?;
            }
            (_, KeyCode::Down) => {
                self.move_history_next()?;
            }
            (_, KeyCode::Left) => {
                self.clear_completion_menu()?;
                self.move_cursor_left()?;
            }
            (_, KeyCode::Right) => {
                self.clear_completion_menu()?;
                self.move_cursor_right()?;
            }
            (_, KeyCode::Tab) if self.completion_menu.is_some() => {}
            (_, KeyCode::Tab) => {
                self.handle_tab(&mut completion_handler)?;
            }
            _ => (),
        }

        Ok(None)
    }

    fn on_char(&mut self, c: char) -> Result<(), ShellError> {
        self.reset_history_navigation();
        let insertion_index = self.cursor;
        self.line.insert(self.cursor, c);
        self.cursor += c.len_utf8();

        eprint!("{}", &self.line[insertion_index..]);
        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, display_width(&self.line[self.cursor..]))
        );
        std::io::stderr().flush()?;

        Ok(())
    }

    fn display_newline() -> Result<(), ShellError> {
        eprintln!();
        std::io::stderr().flush()?;

        Ok(())
    }

    fn clear_screen(&mut self) -> Result<(), ShellError> {
        term::clear_screen()?;
        self.display_prompt()?;
        eprint!("{}", self.line.as_str());
        std::io::stderr().flush()?;
        Ok(())
    }

    #[allow(clippy::string_slice, reason = "it's calculated based on char indices")]
    fn backspace(&mut self) -> Result<(), ShellError> {
        self.reset_history_navigation();
        if self.cursor == 0 {
            return Ok(());
        }

        let deleted_char_start = self.prev_char_boundary(self.cursor);
        let deleted_width = display_width(&self.line[deleted_char_start..self.cursor]);
        self.line.drain(deleted_char_start..self.cursor);
        self.cursor = deleted_char_start;
        let suffix_width = display_width(&self.line[self.cursor..]);

        eprint!("{}", repeated_char_str(BACKSPACE, deleted_width));
        eprint!("{}{}", &self.line[self.cursor..], " ".repeat(deleted_width));
        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, suffix_width + deleted_width)
        );

        std::io::stderr().flush()?;
        Ok(())
    }

    fn move_cursor_left(&mut self) -> Result<(), ShellError> {
        if self.cursor == 0 {
            return Ok(());
        }

        let previous = self.prev_char_boundary(self.cursor);
        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, display_width(&self.line[previous..self.cursor]))
        );
        std::io::stderr().flush()?;
        self.cursor = previous;

        Ok(())
    }

    fn move_cursor_right(&mut self) -> Result<(), ShellError> {
        if self.cursor >= self.line.len() {
            return Ok(());
        }

        let next = self.next_char_boundary(self.cursor);
        eprint!("{}", &self.line[self.cursor..next]);
        std::io::stderr().flush()?;
        self.cursor = next;

        Ok(())
    }

    fn reset_history_navigation(&mut self) {
        self.history_position = None;
        self.history_draft = None;
    }

    fn move_history_previous(&mut self) -> Result<(), ShellError> {
        if self.history_entries.is_empty() {
            return Ok(());
        }

        let next_position = match self.history_position {
            Some(0) => 0,
            Some(position) => position - 1,
            None => {
                self.history_draft = Some(self.line.clone());
                self.history_entries.len() - 1
            }
        };

        self.history_position = Some(next_position);
        self.replace_line(self.history_entries[next_position].clone())
    }

    fn move_history_next(&mut self) -> Result<(), ShellError> {
        let Some(position) = self.history_position else {
            return Ok(());
        };

        if position + 1 < self.history_entries.len() {
            let next_position = position + 1;
            self.history_position = Some(next_position);
            self.replace_line(self.history_entries[next_position].clone())
        } else {
            self.history_position = None;
            let draft = self.history_draft.take().unwrap_or_default();
            self.replace_line(draft)
        }
    }

    fn replace_line(&mut self, line: String) -> Result<(), ShellError> {
        let old_width = display_width(&self.line);
        self.line = line;
        self.cursor = self.line.len();
        let new_width = display_width(&self.line);

        let Some(origin) = self.input_origin else {
            term::clear_current_line()?;
            self.display_prompt()?;
            eprint!("{}", self.line);
            std::io::stderr().flush()?;
            return Ok(());
        };

        let _cursor = term::hide_cursor().ok();
        term::set_cursor_position(origin.x, origin.y)?;
        eprint!("{}", self.line);
        if old_width > new_width {
            eprint!("{}", " ".repeat(old_width - new_width));
        }
        term::set_cursor_position(
            origin
                .x
                .saturating_add(i16::try_from(new_width).unwrap_or(i16::MAX)),
            origin.y,
        )?;
        std::io::stderr().flush()?;
        Ok(())
    }

    fn prev_char_boundary(&self, index: usize) -> usize {
        self.line[..index]
            .char_indices()
            .last()
            .map_or(0, |(idx, _)| idx)
    }

    fn next_char_boundary(&self, index: usize) -> usize {
        let mut iter = self.line[index..].char_indices();
        let _ = iter.next();
        iter.next()
            .map_or(self.line.len(), |(offset, _)| index + offset)
    }

    fn handle_tab(
        &mut self,
        completion_handler: &mut impl FnMut(
            &str,
            usize,
        ) -> Result<
            crate::engine::completion::Completions,
            ShellError,
        >,
    ) -> Result<(), ShellError> {
        if self.completion_menu.is_some() {
            return Ok(());
        }

        let completions = completion_handler(self.line.as_str(), self.cursor)?;
        self.handle_completions(completions)
    }

    fn move_completion_selection(
        &mut self,
        direction: CompletionDirection,
    ) -> Result<(), ShellError> {
        let Some(menu) = self.completion_menu.as_mut() else {
            return Ok(());
        };

        menu.selected = move_completion_selection_index(
            menu.selected,
            menu.completions.candidates.len(),
            menu.rows,
            menu.columns,
            direction,
        );
        self.render_completion_menu()
    }

    fn handle_completions(
        &mut self,
        completions: crate::engine::completion::Completions,
    ) -> Result<(), ShellError> {
        if completions.candidates.is_empty() {
            Ok(())
        } else if completions.candidates.len() == 1 {
            self.handle_single_completion(&completions)
        } else {
            self.completion_menu = Some(CompletionMenu {
                completions,
                selected: 0,
                rows: 1,
                columns: 1,
                rendered_lines: 0,
                cursor_visibility: None,
            });
            self.render_completion_menu()
        }
    }

    #[expect(
        clippy::string_slice,
        reason = "all offsets are expected to be at char boundaries"
    )]
    fn handle_single_completion(
        &mut self,
        completions: &crate::engine::completion::Completions,
    ) -> Result<(), ShellError> {
        let Some(candidate) = completions.candidates.first() else {
            return Ok(());
        };

        if completions.insertion_index + completions.delete_count != self.cursor {
            return Ok(());
        }

        let mut delete_count = completions.delete_count;
        let mut redisplay_offset = completions.insertion_index;

        if delete_count > 0
            && candidate.starts_with(&self.line[redisplay_offset..redisplay_offset + delete_count])
        {
            redisplay_offset += delete_count;
            delete_count = 0;
        }

        let deleted_width = display_width(
            &self.line[completions.insertion_index..completions.insertion_index + delete_count],
        );

        let mut updated_line = self.line.clone();
        updated_line.truncate(completions.insertion_index);
        updated_line.push_str(candidate);
        updated_line.push_str(&self.line[self.cursor..]);
        self.line = updated_line;

        self.cursor = completions.insertion_index + candidate.len();

        let move_left = repeated_char_str(BACKSPACE, deleted_width);
        eprint!("{move_left}{}", &self.line[redisplay_offset..]);

        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, display_width(&self.line[self.cursor..]))
        );

        std::io::stderr().flush()?;

        Ok(())
    }

    fn render_completion_menu(&mut self) -> Result<(), ShellError> {
        let mut input_cursor = term::get_cursor_position()?;
        let (buffer_width, buffer_height) = term::screen_buffer_size()?;
        let terminal_width = usize::try_from(buffer_width).unwrap_or(80).max(1);

        let (rows, columns, column_width, rendered_lines) = {
            let Some(menu) = self.completion_menu.as_mut() else {
                return Ok(());
            };

            if menu.cursor_visibility.is_none() {
                menu.cursor_visibility = term::hide_cursor().ok();
            }

            let column_width = completion_column_width(&menu.completions);
            let columns = usize::min(
                MAX_COMPLETION_COLUMNS,
                usize::max(1, terminal_width / column_width),
            );
            let rows = menu.completions.candidates.len().div_ceil(columns);
            let required_lines = i16::try_from(rows).unwrap_or(i16::MAX);

            menu.rows = rows.max(1);
            menu.columns = columns.max(1);

            if required_lines >= buffer_height {
                return self.handle_multiple_completions_fallback();
            }

            (rows, columns, column_width, menu.rendered_lines)
        };

        let required_lines = i16::try_from(rows).unwrap_or(i16::MAX);
        input_cursor =
            self.ensure_completion_menu_space(input_cursor, required_lines, buffer_height)?;
        let completions_start_y = input_cursor.y.saturating_add(1);

        let max_rendered = rows.max(rendered_lines);
        for row in 0..max_rendered {
            let y = completions_start_y + i16::try_from(row).unwrap_or(i16::MAX);
            term::set_cursor_position(0, y)?;
            term::clear_current_line()?;
        }

        let Some(menu) = self.completion_menu.as_mut() else {
            return Ok(());
        };

        for row in 0..rows {
            let y = completions_start_y + i16::try_from(row).unwrap_or(i16::MAX);
            term::set_cursor_position(0, y)?;

            for column in 0..columns {
                let index = column * rows + row;
                let Some(candidate) = menu.completions.candidates.get(index) else {
                    continue;
                };

                let formatted =
                    format_completion_candidate(candidate.as_str(), &menu.completions.options);
                let selected = index == menu.selected;
                let display_value = if selected {
                    formatted.to_ascii_uppercase()
                } else {
                    formatted
                };
                let marker = if selected { '>' } else { ' ' };
                let cell = format_completion_cell(marker, &display_value, column_width);
                eprint!("{cell}");
            }
        }

        menu.rendered_lines = rows;
        term::set_cursor_position(input_cursor.x, input_cursor.y)?;
        std::io::stderr().flush()?;

        Ok(())
    }

    fn ensure_completion_menu_space(
        &mut self,
        input_cursor: term::CursorPosition,
        required_lines: i16,
        buffer_height: i16,
    ) -> Result<term::CursorPosition, ShellError> {
        let menu_end_y = input_cursor
            .y
            .saturating_add(1)
            .saturating_add(required_lines);
        if menu_end_y <= buffer_height {
            return Ok(input_cursor);
        }

        let lines_to_move_up = menu_end_y.saturating_sub(buffer_height);
        let lines_to_bottom = buffer_height
            .saturating_sub(1)
            .saturating_sub(input_cursor.y);
        let newlines = lines_to_bottom.saturating_add(lines_to_move_up);

        for _ in 0..newlines {
            eprintln!();
        }
        std::io::stderr().flush()?;

        let input_y = input_cursor.y.saturating_sub(lines_to_move_up);
        term::set_cursor_position(0, input_y)?;
        term::clear_current_line()?;
        self.display_prompt()?;
        eprint!(
            "{}{}",
            self.line,
            repeated_char_str(BACKSPACE, display_width(&self.line[self.cursor..]))
        );
        std::io::stderr().flush()?;

        term::get_cursor_position().map_err(ShellError::from)
    }

    fn handle_multiple_completions_fallback(&mut self) -> Result<(), ShellError> {
        let Some(menu) = &self.completion_menu else {
            return Ok(());
        };

        Self::display_newline()?;
        for (index, candidate) in menu.completions.candidates.iter().enumerate() {
            let formatted =
                format_completion_candidate(candidate.as_str(), &menu.completions.options);
            let selected = index == menu.selected;
            let display_value = if selected {
                formatted.to_ascii_uppercase()
            } else {
                formatted
            };
            let marker = if selected { '>' } else { ' ' };
            eprintln!("{marker}{display_value}");
        }
        std::io::stderr().flush()?;

        self.display_prompt()?;
        eprint!(
            "{}{}",
            self.line,
            repeated_char_str(BACKSPACE, display_width(&self.line[self.cursor..]))
        );
        std::io::stderr().flush()?;

        Ok(())
    }

    fn clear_completion_menu(&mut self) -> Result<(), ShellError> {
        let Some(menu) = self.completion_menu.take() else {
            return Ok(());
        };

        let input_cursor = term::get_cursor_position()?;
        let start_y = input_cursor.y.saturating_add(1);

        for row in 0..menu.rendered_lines {
            let y = start_y + i16::try_from(row).unwrap_or(i16::MAX);
            term::set_cursor_position(0, y)?;
            term::clear_current_line()?;
        }

        term::set_cursor_position(input_cursor.x, input_cursor.y)?;
        std::io::stderr().flush()?;
        Ok(())
    }

    fn accept_completion_selection(&mut self) -> Result<(), ShellError> {
        let Some(menu) = &self.completion_menu else {
            return Ok(());
        };
        let Some(candidate) = menu.completions.candidates.get(menu.selected).cloned() else {
            return Ok(());
        };
        let insertion_index = menu.completions.insertion_index;
        let delete_count = menu.completions.delete_count;

        self.clear_completion_menu()?;
        self.apply_completion(insertion_index, delete_count, candidate)
    }

    #[expect(
        clippy::string_slice,
        reason = "all offsets are expected to be at char boundaries"
    )]
    fn apply_completion(
        &mut self,
        insertion_index: usize,
        delete_count: usize,
        candidate: String,
    ) -> Result<(), ShellError> {
        if insertion_index + delete_count != self.cursor {
            return Ok(());
        }

        let mut updated_line = self.line.clone();
        updated_line.truncate(insertion_index);
        updated_line.push_str(&candidate);
        updated_line.push_str(&self.line[self.cursor..]);
        self.line = updated_line;
        self.cursor = insertion_index + candidate.len();

        term::clear_current_line()?;
        self.display_prompt()?;
        eprint!(
            "{}{}",
            self.line,
            repeated_char_str(BACKSPACE, display_width(&self.line[self.cursor..]))
        );
        std::io::stderr().flush()?;
        Ok(())
    }
}

#[allow(clippy::string_slice)]
fn format_completion_candidate(
    mut candidate: &str,
    options: &crate::engine::completion::ProcessingOptions,
) -> String {
    if options.treat_as_filenames {
        let trimmed = crate::engine::sys::fs::strip_path_separator_suffix(candidate);
        if let Some(index) = crate::engine::sys::fs::rfind_path_separator(trimmed) {
            candidate = &candidate[index + 1..];
        }
    }

    candidate.to_string()
}

fn completion_column_width(completions: &crate::engine::completion::Completions) -> usize {
    let max_candidate_width = completions
        .candidates
        .iter()
        .map(|candidate| {
            display_width(&format_completion_candidate(
                candidate.as_str(),
                &completions.options,
            ))
        })
        .max()
        .unwrap_or(1);

    max_candidate_width + 2
}

fn format_completion_cell(marker: char, value: &str, width: usize) -> String {
    let available = width.saturating_sub(2);
    let mut value = truncate_to_display_width(value, available);
    let value_width = display_width(&value);
    if value_width < available {
        value.push_str(&" ".repeat(available - value_width));
    }
    format!("{marker}{value} ")
}

fn move_completion_selection_index(
    selected: usize,
    candidate_count: usize,
    rows: usize,
    columns: usize,
    direction: CompletionDirection,
) -> usize {
    if candidate_count == 0 {
        return 0;
    }

    let rows = rows.max(1);
    let columns = columns.max(1);
    let selected = selected.min(candidate_count - 1);
    let row = selected % rows;
    let column = selected / rows;

    match direction {
        CompletionDirection::Up => row
            .checked_sub(1)
            .map_or(selected, |row| column * rows + row),
        CompletionDirection::Down => {
            let next = selected + 1;
            if row + 1 < completion_column_len(candidate_count, rows, column) {
                next
            } else {
                selected
            }
        }
        CompletionDirection::Left => {
            let Some(column) = column.checked_sub(1) else {
                return selected;
            };
            let row = row.min(completion_column_len(candidate_count, rows, column) - 1);
            column * rows + row
        }
        CompletionDirection::Right => {
            if column + 1 >= columns {
                return selected;
            }
            let column = column + 1;
            let column_len = completion_column_len(candidate_count, rows, column);
            if column_len == 0 {
                selected
            } else {
                let row = row.min(column_len - 1);
                column * rows + row
            }
        }
    }
}

fn completion_column_len(candidate_count: usize, rows: usize, column: usize) -> usize {
    candidate_count.saturating_sub(column * rows).min(rows)
}

fn truncate_to_display_width(value: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;
    for ch in value.chars() {
        let ch_width = char_display_width(ch);
        if width + ch_width > max_width {
            break;
        }
        result.push(ch);
        width += ch_width;
    }
    result
}

fn display_width(value: &str) -> usize {
    value.chars().map(char_display_width).sum()
}

fn char_display_width(ch: char) -> usize {
    if ch.is_control() || is_combining_mark(ch) {
        0
    } else if is_wide_char(ch) {
        2
    } else {
        1
    }
}

fn is_combining_mark(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0300..=0x036F
            | 0x1AB0..=0x1AFF
            | 0x1DC0..=0x1DFF
            | 0x20D0..=0x20FF
            | 0xFE20..=0xFE2F
    )
}

fn is_wide_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x1100..=0x115F
            | 0x2329..=0x232A
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
    )
}

fn repeated_char_str(c: char, count: usize) -> String {
    (0..count).map(|_| c).collect()
}

#[cfg(test)]
mod tests {
    use super::{CompletionDirection, move_completion_selection_index};

    #[test]
    fn completion_selection_moves_inside_visible_grid() {
        let rows = 3;
        let columns = 4;

        assert_eq!(
            move_completion_selection_index(0, 10, rows, columns, CompletionDirection::Down),
            1
        );
        assert_eq!(
            move_completion_selection_index(1, 10, rows, columns, CompletionDirection::Up),
            0
        );
        assert_eq!(
            move_completion_selection_index(1, 10, rows, columns, CompletionDirection::Right),
            4
        );
        assert_eq!(
            move_completion_selection_index(4, 10, rows, columns, CompletionDirection::Left),
            1
        );
    }

    #[test]
    fn completion_selection_clamps_at_grid_edges() {
        let rows = 3;
        let columns = 4;

        assert_eq!(
            move_completion_selection_index(0, 10, rows, columns, CompletionDirection::Up),
            0
        );
        assert_eq!(
            move_completion_selection_index(2, 10, rows, columns, CompletionDirection::Down),
            2
        );
        assert_eq!(
            move_completion_selection_index(0, 10, rows, columns, CompletionDirection::Left),
            0
        );
        assert_eq!(
            move_completion_selection_index(9, 10, rows, columns, CompletionDirection::Right),
            9
        );
    }

    #[test]
    fn completion_selection_right_clamps_to_short_last_column() {
        let rows = 3;
        let columns = 4;

        assert_eq!(
            move_completion_selection_index(8, 10, rows, columns, CompletionDirection::Right),
            9
        );
    }
}
