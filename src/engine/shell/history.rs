//! History management for shells.

use std::path::PathBuf;

use crate::engine::{error, openfiles};

impl crate::engine::Shell {
    pub(super) fn load_history(
        &self,
    ) -> Result<Option<crate::engine::history::History>, error::Error> {
        const MAX_FILE_SIZE_FOR_HISTORY_IMPORT: u64 = 64 * 1024 * 1024;

        let Some(history_path) = self.history_file_path() else {
            return Ok(None);
        };

        let mut options = std::fs::File::options();
        options.read(true);

        let load_result = (|| -> Result<Option<crate::engine::history::History>, error::Error> {
            let mut history_file =
                match self.open_file(&options, &history_path, &self.default_exec_params()) {
                    Ok(history_file) => history_file,
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(err) => return Err(err.into()),
                };

            // 根据文件大小快速处理空文件并拒绝不合理的大文件.
            if let openfiles::OpenFile::File(file) = &mut history_file {
                let file_size = file.metadata()?.len();
                if file_size == 0 {
                    return Ok(None);
                }
                if file_size > MAX_FILE_SIZE_FOR_HISTORY_IMPORT {
                    return Err(error::ErrorKind::HistoryFileTooLargeToImport.into());
                }
            }

            Ok(Some(crate::engine::history::History::import_with_limit(
                history_file,
                self.history_item_limit(),
            )?))
        })();

        if let Err(err) = &load_result {
            log::warn!(
                "couldn't load history from '{}': {err}",
                history_path.display()
            );
        }

        load_result
    }

    fn history_item_limit(&self) -> Option<usize> {
        const DEFAULT_HISTORY_ITEM_LIMIT: usize = 10_000;

        let Some(value) = self.env_str("HISTSIZE") else {
            return Some(DEFAULT_HISTORY_ITEM_LIMIT);
        };
        let Ok(value) = value.parse::<i64>() else {
            return Some(DEFAULT_HISTORY_ITEM_LIMIT);
        };

        usize::try_from(value).ok()
    }

    /// Returns the path to the history file used by the shell, if one is set.
    pub fn history_file_path(&self) -> Option<PathBuf> {
        self.env_str("HISTFILE")
            .map(|s| PathBuf::from(s.into_owned()))
    }

    /// Returns the path to the history file used by the shell, if one is set.
    pub fn history_time_format(&self) -> Option<String> {
        self.env_str("HISTTIMEFORMAT").map(|s| s.into_owned())
    }

    /// Saves history back to any backing storage.
    pub fn save_history(&mut self) -> Result<(), error::Error> {
        let Some(history_file_path) = self.history_file_path() else {
            return Ok(());
        };
        let history_file_path = self.absolute_path(&history_file_path);
        let append = self.options.append_to_history_file;
        let write_timestamps = self.env.is_set("HISTTIMEFORMAT");

        if let Some(history) = &mut self.history {
            history.flush(history_file_path, append, append, write_timestamps)?;
        }

        Ok(())
    }

    /// Adds a command to history.
    pub fn add_to_history(&mut self, command: &str) -> Result<(), error::Error> {
        let history_item_limit = self.history_item_limit();
        if let Some(history) = &mut self.history {
            // Trim.
            let command = command.trim();

            // For now, discard empty commands.
            if command.is_empty() {
                return Ok(());
            }

            // Add it to history.
            history.add(crate::engine::history::Item {
                id: 0,
                command_line: command.to_owned(),
                timestamp: Some(nanotime::NanoTime::now_utc()),
                dirty: true,
            })?;
            if let Some(history_item_limit) = history_item_limit {
                history.truncate_to_max_items(history_item_limit);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{Shell, ShellVariable, history};

    fn shell_with_history_file(path: &std::path::Path) -> Shell {
        let mut shell = Shell::empty();
        shell.history = Some(history::History::import("old\n".as_bytes()).unwrap());
        shell
            .history
            .as_mut()
            .unwrap()
            .add(history::Item::new("new"))
            .unwrap();
        shell
            .set_env_global(
                "HISTFILE",
                ShellVariable::new(path.to_string_lossy().to_string()),
            )
            .unwrap();
        shell
    }

    #[test]
    fn missing_history_file_is_treated_as_empty_history() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("missing-history");
        let mut shell = Shell::empty();
        shell
            .set_env_global(
                "HISTFILE",
                ShellVariable::new(path.to_string_lossy().to_string()),
            )
            .unwrap();

        assert!(shell.load_history().unwrap().is_none());
    }

    #[compio::test]
    async fn history_load_errors_remain_nonfatal_to_shell_creation() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let path = scratch.path().join("history-directory");
        std::fs::create_dir(&path)?;

        let shell = Shell::builder()
            .interactive(true)
            .var(
                "HISTFILE",
                ShellVariable::new(path.to_string_lossy().to_string()),
            )
            .build()
            .await?;

        assert_eq!(shell.history().unwrap().count(), 0);
        Ok(())
    }

    #[test]
    fn negative_histsize_is_unlimited_and_nonnegative_values_are_preserved() {
        let mut shell = Shell::empty();

        shell
            .set_env_global("HISTSIZE", ShellVariable::new("-1"))
            .unwrap();
        assert_eq!(shell.history_item_limit(), None);

        shell
            .set_env_global("HISTSIZE", ShellVariable::new("0"))
            .unwrap();
        assert_eq!(shell.history_item_limit(), Some(0));

        shell
            .set_env_global("HISTSIZE", ShellVariable::new("100001"))
            .unwrap();
        assert_eq!(shell.history_item_limit(), Some(100_001));
    }

    #[test]
    fn save_history_appends_only_dirty_items_when_histappend_is_enabled() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("history");
        std::fs::write(&path, "external\n").unwrap();
        let mut shell = shell_with_history_file(&path);
        shell.options.append_to_history_file = true;

        shell.save_history().unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "external\nnew\n");
        assert!(!shell.history().unwrap().get(1).unwrap().dirty);
    }

    #[test]
    fn save_history_overwrites_all_items_when_histappend_is_disabled() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("history");
        std::fs::write(&path, "external\n").unwrap();
        let mut shell = shell_with_history_file(&path);
        shell.options.append_to_history_file = false;

        shell.save_history().unwrap();

        assert_eq!(std::fs::read_to_string(path).unwrap(), "old\nnew\n");
        assert!(!shell.history().unwrap().get(1).unwrap().dirty);
    }
}
