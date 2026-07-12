use crate::engine::{ExecutionResult, builtins, error, history};
use std::io::Write;

/// Process command history list.
pub(crate) struct FcCommand {
    /// List commands instead of editing them.
    list: bool,

    /// Suppress line numbers when listing.
    no_line_numbers: bool,

    /// Reverse the order of commands.
    reverse: bool,

    /// Re-execute command after substitution (old=new format).
    substitute: bool,

    /// Editor to use (only relevant when not listing or substituting).
    editor: Option<String>,

    /// First command in range (number or string prefix).
    first: Option<String>,

    /// Last command in range (number or string prefix).
    last: Option<String>,
}

impl builtins::Command for FcCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            list: false,
            no_line_numbers: false,
            reverse: false,
            substitute: false,
            editor: None,
            first: None,
            last: None,
        };
        let mut args = builtins::BuiltinArgs::new(args);
        let mut positionals = Vec::new();

        while let Some(arg) = args.next_arg() {
            if arg == "--" {
                positionals.extend(args.rest());
                break;
            }

            let Some(flags) = arg.strip_prefix('-') else {
                positionals.push(arg);
                positionals.extend(args.rest());
                break;
            };

            if flags.is_empty() || flags.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                positionals.push(arg);
                positionals.extend(args.rest());
                break;
            }

            for (idx, flag) in flags.char_indices() {
                match flag {
                    'l' => command.list = true,
                    'n' => command.no_line_numbers = true,
                    'r' => command.reverse = true,
                    's' => command.substitute = true,
                    'e' => {
                        let value_start = idx + flag.len_utf8();
                        let value = if value_start < flags.len() {
                            flags[value_start..].to_owned()
                        } else {
                            args.next_value("-e")?
                        };
                        command.editor = Some(value);
                        break;
                    }
                    _ => return Err(format!("-{flag}: invalid option")),
                }
            }
        }

        if command.no_line_numbers && !command.list {
            return Err(String::from("-n requires -l"));
        }

        if positionals.len() > 2 {
            return Err(String::from("too many arguments"));
        }

        command.first = positionals.first().cloned();
        command.last = positionals.get(1).cloned();

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        if self.substitute {
            return self.do_execute(context).await;
        }

        if self.list {
            return self.do_list(&context);
        }

        error::unimp("fc editor mode is not yet implemented")
    }
}

impl FcCommand {
    fn do_list(
        &self,
        context: &crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        let history = context.shell.history().ok_or_else(|| {
            crate::engine::Error::from(crate::engine::ErrorKind::HistoryNotEnabled)
        })?;

        let (first_idx, last_idx, reverse) = self.resolve_range(history)?;

        if reverse {
            for idx in (first_idx..=last_idx).rev() {
                self.write_history_item(context, history, idx)?;
            }
        } else {
            for idx in first_idx..=last_idx {
                self.write_history_item(context, history, idx)?;
            }
        }

        Ok(ExecutionResult::success())
    }

    async fn do_execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        let history = context.shell.history().ok_or_else(|| {
            crate::engine::Error::from(crate::engine::ErrorKind::HistoryNotEnabled)
        })?;

        // Parse the first argument for pattern=replacement
        let (pattern, replacement) = self
            .first
            .as_ref()
            .and_then(|s| s.split_once('='))
            .map_or((None, None), |(p, r)| (Some(p), Some(r)));

        // Determine which command to re-execute
        let cmd_spec = if pattern.is_some() {
            // If we have a pattern, the command spec is in 'last' if present
            self.last.as_deref()
        } else {
            // Otherwise, it's in 'first'
            self.first.as_deref()
        };

        // Find the command
        let cmd_line = if let Some(spec) = cmd_spec {
            Self::find_command_by_specifier(history, spec)?
        } else {
            // No spec means use the previous command (excluding the fc command itself)
            let effective_count = effective_history_count(history);
            history
                .get(effective_count.saturating_sub(1))
                .map(|item| item.command_line.clone())
                .ok_or_else(|| crate::engine::Error::from(error::ErrorKind::HistoryItemNotFound))?
        };

        // Apply substitution if present
        let final_cmd = if let (Some(pat), Some(rep)) = (pattern, replacement) {
            cmd_line.replace(pat, rep)
        } else {
            cmd_line
        };

        // Echo the command to stderr.
        writeln!(context.stderr(), "{final_cmd}")?;

        // Remove the fc command from history before executing the substituted command
        // This matches bash behavior where the fc command is replaced by the executed command
        let history_mut = context.shell.history_mut().ok_or_else(|| {
            crate::engine::Error::from(crate::engine::ErrorKind::HistoryNotEnabled)
        })?;
        history_mut.remove_nth_item(history_mut.count().saturating_sub(1));

        let source_info = crate::engine::SourceInfo::from("(history)");

        // Execute the command
        let result = context
            .shell
            .run_string(final_cmd.clone(), &source_info, &context.params)
            .await?;

        // Add the executed command to history.
        context.shell.add_to_history(&final_cmd)?;

        Ok(result)
    }

    fn write_history_item(
        &self,
        context: &crate::engine::ExecutionContext<'_>,
        history: &history::History,
        idx: usize,
    ) -> Result<(), crate::engine::Error> {
        if let Some(item) = history.get(idx) {
            if self.no_line_numbers {
                // With -n, bash still outputs a tab before the command
                writeln!(context.stdout(), "\t {}", item.command_line)?;
            } else {
                // Match bash's fc format: number, tab, command
                writeln!(context.stdout(), "{}\t {}", idx + 1, item.command_line)?;
            }
        }

        Ok(())
    }

    fn resolve_range(
        &self,
        history: &history::History,
    ) -> Result<(usize, usize, bool), crate::engine::Error> {
        let effective_count = effective_history_count(history);
        let max_idx = effective_count.saturating_sub(1);

        // Resolve first index
        let first_idx = self
            .first
            .as_ref()
            .map(|s| Self::resolve_position(history, s))
            .transpose()?
            .unwrap_or_else(|| {
                if self.list {
                    effective_count.saturating_sub(16) // Default for listing: -16
                } else {
                    max_idx // Default for editing: previous command
                }
            });

        // Resolve last index (default depends on mode and first_idx)
        let default_last = if self.list { max_idx } else { first_idx };
        let last_idx = self
            .last
            .as_ref()
            .map(|s| Self::resolve_position(history, s))
            .transpose()?
            .unwrap_or(default_last);

        // If first > last, swap them and indicate reversal
        let (first_idx, last_idx, force_reverse) = if first_idx > last_idx {
            (last_idx, first_idx, true)
        } else {
            (first_idx, last_idx, false)
        };

        // Clamp both indices to valid range
        Ok((
            first_idx.min(max_idx),
            last_idx.min(max_idx),
            force_reverse || self.reverse,
        ))
    }

    /// Resolves a position specifier (number or string prefix) to a history index.
    /// NOTE: The returned index may still be out of range if the history is empty.
    ///
    /// # Arguments
    ///
    /// * `history` - The history to resolve against.
    /// * `spec` - The position specifier (number or string prefix).
    fn resolve_position(
        history: &history::History,
        spec: &str,
    ) -> Result<usize, crate::engine::Error> {
        // Try to parse it as a number. If it's not parseable, then we need to assume
        // it's a string prefix we need to search for.
        let Ok(num) = spec.parse::<i64>() else {
            // Not a number, treat as string prefix
            return Self::find_command_by_prefix(history, spec);
        };

        let effective_count = effective_history_count(history);

        #[expect(clippy::cast_sign_loss)]
        #[expect(clippy::cast_possible_truncation)]
        let result = match num.cmp(&0) {
            std::cmp::Ordering::Equal => {
                // 0 means -1 for listing (relative to effective count)
                effective_count.saturating_sub(1)
            }
            std::cmp::Ordering::Greater => {
                let idx = (num - 1) as usize;
                if idx >= effective_count {
                    return Err(error::ErrorKind::HistoryItemNotFound.into());
                }
                idx
            }
            std::cmp::Ordering::Less => {
                let offset = num.unsigned_abs() as usize;
                if offset > effective_count {
                    return Err(error::ErrorKind::HistoryItemNotFound.into());
                }
                effective_count - offset
            }
        };

        Ok(result)
    }

    /// Finds the command matching the given specifier (number or string prefix). Returns
    /// the command line. Returns an error if no such command can be found in the history.
    ///
    /// # Arguments
    ///
    /// * `history` - The history to search.
    /// * `spec` - The position spec
    fn find_command_by_specifier(
        history: &history::History,
        spec: &str,
    ) -> Result<String, crate::engine::Error> {
        let idx = Self::resolve_position(history, spec)?;
        history
            .get(idx)
            .map(|item| item.command_line.clone())
            .ok_or_else(|| crate::engine::Error::from(error::ErrorKind::HistoryItemNotFound))
    }

    /// Finds the most recent command starting with the given prefix. Returns
    /// the index of the command in the history. Returns an error if no such
    /// command can be found in the history.
    ///
    /// # Arguments
    ///
    /// * `history` - The history to search.
    /// * `prefix` - The command prefix to search for.
    fn find_command_by_prefix(
        history: &history::History,
        prefix: &str,
    ) -> Result<usize, crate::engine::Error> {
        // Search backwards for a command starting with the prefix (excluding fc command itself)
        let effective_count = effective_history_count(history);

        for idx in (0..effective_count).rev() {
            if let Some(item) = history.get(idx)
                && item.command_line.starts_with(prefix)
            {
                return Ok(idx);
            }
        }

        Err(crate::engine::Error::from(
            error::ErrorKind::HistoryItemNotFound,
        ))
    }
}

/// Returns the effective history count (excluding the fc command itself).
fn effective_history_count(history: &history::History) -> usize {
    history.count().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_history() -> history::History {
        let mut history = history::History::default();
        for command in ["echo first", "echo second", "fc -l"] {
            history.add(history::Item::new(command)).unwrap();
        }
        history
    }

    #[test]
    fn out_of_range_position_does_not_fall_back_to_first_item() {
        let history = sample_history();
        assert!(FcCommand::resolve_position(&history, "99").is_err());
        assert!(FcCommand::resolve_position(&history, "-99").is_err());
        assert_eq!(FcCommand::resolve_position(&history, "1").unwrap(), 0);
    }
}
