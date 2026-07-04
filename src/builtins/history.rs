use crate::engine::{ExecutionResult, builtins, error, history};
use std::{io::Write, path::PathBuf};

/// Query or manipulate the shell's command history.
// TODO(history): Evaluate which of the options conflict with each other.
#[expect(clippy::option_option)]
pub(crate) struct HistoryCommand {
    /// Clears all history.
    clear_history: bool,

    /// Deletes the history entry at the given offset. Positive offsets are relative to the
    /// beginning of the history, while negative offsets are relative to the end of the history.
    delete_offset: Option<i64>,

    /// Appends the history from the current session to the history file.
    append_session_to_file: Option<Option<String>>,

    /// Appends any remaining history from the history file to the current session.
    append_rest_of_file_to_session: Option<Option<String>>,

    /// Appends the history from the history file to the current session.
    append_file_to_session: Option<Option<String>>,

    /// Replaces the history file with the current session history.
    write_session_to_file: Option<Option<String>>,

    /// History-expands positional arguments and displays them.
    expand_args: Option<Vec<String>>,

    /// Appends positional arguments as an entry in the current session.
    append_args_to_session: Option<Vec<String>>,

    /// Arguments.
    args: Vec<String>,
}

struct HistoryConfig {
    default_history_file_path: Option<PathBuf>,
    time_format: Option<String>,
}

impl builtins::Command for HistoryCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            clear_history: false,
            delete_offset: None,
            append_session_to_file: None,
            append_rest_of_file_to_session: None,
            append_file_to_session: None,
            write_session_to_file: None,
            expand_args: None,
            append_args_to_session: None,
            args: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);

        while let Some(arg) = args.next_arg() {
            if arg == "--" {
                command.args.extend(args.rest());
                break;
            }

            let Some(flags) = arg.strip_prefix('-') else {
                command.args.push(arg);
                command.args.extend(args.rest());
                break;
            };

            if flags.is_empty() {
                command.args.push(arg);
                command.args.extend(args.rest());
                break;
            }

            for (idx, flag) in flags.char_indices() {
                let value_start = idx + flag.len_utf8();
                let attached_value =
                    (value_start < flags.len()).then(|| flags[value_start..].to_owned());

                match flag {
                    'c' => command.clear_history = true,
                    'd' => {
                        let value = attached_value.unwrap_or_default();
                        let value = if value.is_empty() {
                            args.next_value("-d")?
                        } else {
                            value
                        };
                        command.delete_offset = Some(
                            value
                                .parse()
                                .map_err(|_| format!("-d: invalid offset: {value}"))?,
                        );
                        break;
                    }
                    'a' => {
                        command.append_session_to_file =
                            Some(take_optional_history_file(&mut args));
                    }
                    'n' => {
                        command.append_rest_of_file_to_session =
                            Some(take_optional_history_file(&mut args));
                    }
                    'r' => {
                        command.append_file_to_session =
                            Some(take_optional_history_file(&mut args));
                    }
                    'w' => {
                        command.write_session_to_file = Some(take_optional_history_file(&mut args));
                    }
                    'p' => {
                        let mut values = Vec::new();
                        if let Some(value) = attached_value {
                            values.push(value);
                        }
                        values.extend(args.rest());
                        command.expand_args = Some(values);
                        return Ok(command);
                    }
                    's' => {
                        let mut values = Vec::new();
                        if let Some(value) = attached_value {
                            values.push(value);
                        }
                        values.extend(args.rest());
                        command.append_args_to_session = Some(values);
                        return Ok(command);
                    }
                    _ => return Err(format!("-{flag}: invalid option")),
                }
            }
        }

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        // Retrieve the shell's history config while we still can.
        let config = HistoryConfig {
            default_history_file_path: context.shell.history_file_path(),
            time_format: context.shell.history_time_format(),
        };

        let stdout = context.stdout();
        let stderr = context.stderr();

        if let Some(history) = context.shell.history_mut() {
            self.execute_with_history(history, config, stdout, stderr)
        } else {
            Err(crate::engine::ErrorKind::HistoryNotEnabled.into())
        }
    }
}

impl HistoryCommand {
    #[expect(clippy::cast_possible_wrap)]
    #[expect(clippy::cast_possible_truncation)]
    #[expect(clippy::cast_sign_loss)]
    fn execute_with_history(
        &self,
        history: &mut history::History,
        config: HistoryConfig,
        stdout: impl Write,
        mut stderr: impl Write,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        if self.clear_history {
            history.clear()?;
        }

        if let Some(offset) = self.delete_offset {
            if offset == 0 {
                writeln!(stderr, "cannot delete history item at offset 0")?;
                return Ok(ExecutionResult::invalid_usage());
            }

            if offset > 0 {
                // Convert to 0-based index.
                let index = (offset - 1) as usize;
                if !history.remove_nth_item(index) {
                    writeln!(stderr, "index past end of history")?;
                    return Ok(ExecutionResult::invalid_usage());
                }
            } else {
                let count = history.count() as i64;
                let index = count + offset;
                if index < 0 {
                    writeln!(stderr, "index before beginning of history")?;
                    return Ok(ExecutionResult::invalid_usage());
                }

                let _ = history.remove_nth_item(index as usize);
            }

            return Ok(ExecutionResult::success());
        }

        if let Some(append_option) = &self.append_session_to_file {
            if let Some(file_path) = get_effective_history_file_path(
                config.default_history_file_path,
                append_option.as_ref(),
            ) {
                history.flush(
                    file_path,
                    true,                         /* append? */
                    true,                         /* unsaved items only */
                    config.time_format.is_some(), /* write timestamps? */
                )?;
            }

            return Ok(ExecutionResult::success());
        }

        if self.append_rest_of_file_to_session.is_some() {
            return error::unimp("history -n is not yet implemented");
        }

        if self.append_file_to_session.is_some() {
            return error::unimp("history -r is not yet implemented");
        }

        if let Some(write_option) = &self.write_session_to_file {
            if let Some(file_path) = get_effective_history_file_path(
                config.default_history_file_path,
                write_option.as_ref(),
            ) {
                history.flush(
                    file_path,
                    false,                        /* append? */
                    false,                        /* unsaved items only? */
                    config.time_format.is_some(), /* write timestamps? */
                )?;
            }

            return Ok(ExecutionResult::success());
        }

        if self.expand_args.is_some() {
            return error::unimp("history -p is not yet implemented");
        }

        if let Some(args) = &self.append_args_to_session {
            history.add(history::Item::new(args.join(" ")))?;
            return Ok(ExecutionResult::success());
        }

        let max_entries: Option<usize> = if let Some(arg) = self.args.first() {
            Some(crate::engine::int_utils::parse(arg.as_str(), 10)?)
        } else {
            None
        };

        display_history(history, &config, max_entries, stdout, stderr)?;

        Ok(ExecutionResult::success())
    }
}

fn display_history(
    history: &history::History,
    config: &HistoryConfig,
    max_entries: Option<usize>,
    mut stdout: impl Write,
    _stderr: impl Write,
) -> Result<(), crate::engine::Error> {
    let item_count = history.count();
    let skip_count = item_count - max_entries.unwrap_or(item_count);

    for (i, item) in history.iter().skip(skip_count).enumerate() {
        let mut formatted_timestamp = String::new();

        if let Some(timestamp) = item.timestamp
            && let Some(time_format) = &config.time_format
        {
            formatted_timestamp =
                crate::engine::timefmt::format_strftime_subset(&timestamp, time_format);
        }

        // Output format is something like:
        //     1  echo hello world
        std::writeln!(
            stdout,
            "{:>5}  {formatted_timestamp}{}",
            skip_count + i + 1,
            item.command_line
        )?;
    }

    Ok(())
}

fn get_effective_history_file_path(
    default_history_file_path: Option<PathBuf>,
    option: Option<&String>,
) -> Option<PathBuf> {
    option.map_or_else(
        || default_history_file_path,
        |file_path| Some(PathBuf::from(file_path)),
    )
}

fn take_optional_history_file(args: &mut builtins::BuiltinArgs) -> Option<String> {
    if args
        .peek()
        .is_some_and(|arg| arg == "-" || !arg.starts_with('-'))
    {
        args.next_arg()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use pretty_assertions::{assert_eq, assert_matches};

    #[test]
    fn test_parse_dash_a() -> Result<()> {
        let cmd = <HistoryCommand as builtins::Command>::new(["history", "5"].map(String::from))
            .map_err(anyhow::Error::msg)?;
        assert_matches!(cmd.append_session_to_file, None);

        let cmd = <HistoryCommand as builtins::Command>::new(["history", "-a"].map(String::from))
            .map_err(anyhow::Error::msg)?;
        assert_matches!(cmd.append_session_to_file, Some(None));

        let cmd = <HistoryCommand as builtins::Command>::new(
            ["history", "-a", "token"].map(String::from),
        )
        .map_err(anyhow::Error::msg)?;
        assert_eq!(
            cmd.append_session_to_file,
            Some(Some(String::from("token")))
        );

        Ok(())
    }
}
