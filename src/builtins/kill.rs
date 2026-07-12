use std::io::Write;
use std::process::Command;

use crate::engine::{ExecutionResult, builtins};

/// Signal a job or process.
pub(crate) struct KillCommand {
    /// Name of the signal to send.
    signal_name: Option<String>,

    /// Number of the signal to send.
    signal_number: Option<usize>,

    /// List signal names.
    list_signals: bool,

    /// PIDs to terminate.
    args: Vec<String>,
}

impl builtins::Command for KillCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            signal_name: None,
            signal_number: None,
            list_signals: false,
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

            if flags.chars().all(|ch| ch.is_ascii_digit()) {
                command.signal_number = Some(
                    flags
                        .parse()
                        .map_err(|_| format!("invalid signal number: {flags}"))?,
                );
                command.signal_name = None;
                continue;
            }

            if is_known_signal_name(flags) {
                command.signal_name = Some(flags.to_owned());
                command.signal_number = None;
                continue;
            }

            for (idx, flag) in flags.char_indices() {
                match flag {
                    'l' => command.list_signals = true,
                    's' => {
                        let value_start = idx + flag.len_utf8();
                        let value = if value_start < flags.len() {
                            flags[value_start..].to_owned()
                        } else {
                            args.next_value("-s")?
                        };
                        command.signal_name = Some(value);
                        command.signal_number = None;
                        break;
                    }
                    'n' => {
                        let value_start = idx + flag.len_utf8();
                        let value = if value_start < flags.len() {
                            flags[value_start..].to_owned()
                        } else {
                            args.next_value("-n")?
                        };
                        command.signal_number = Some(
                            value
                                .parse()
                                .map_err(|_| format!("-n: invalid signal number: {value}"))?,
                        );
                        command.signal_name = None;
                        break;
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
        if self.list_signals {
            return print_signals(context.stdout());
        }

        if let Some(signal_name) = &self.signal_name {
            if !is_known_signal_name(signal_name) {
                writeln!(
                    context.stderr(),
                    "{}: invalid signal name: {}",
                    context.command_name,
                    signal_name
                )?;
                return Ok(ExecutionResult::invalid_usage());
            }
            if !is_supported_signal_name(signal_name) {
                writeln!(
                    context.stderr(),
                    "{}: signal {} is not supported on Windows",
                    context.command_name,
                    signal_name
                )?;
                return Ok(ExecutionResult::invalid_usage());
            }
        }

        if let Some(signal_number) = self.signal_number {
            if !is_known_signal_number(signal_number) {
                writeln!(
                    context.stderr(),
                    "{}: invalid signal number: {}",
                    context.command_name,
                    signal_number
                )?;
                return Ok(ExecutionResult::invalid_usage());
            }
            if !is_supported_signal_number(signal_number) {
                writeln!(
                    context.stderr(),
                    "{}: signal {} is not supported on Windows",
                    context.command_name,
                    signal_number
                )?;
                return Ok(ExecutionResult::invalid_usage());
            }
        }

        if self.args.is_empty() {
            writeln!(context.stderr(), "{}: invalid usage", context.command_name)?;
            return Ok(ExecutionResult::invalid_usage());
        }

        let force = self
            .signal_name
            .as_deref()
            .is_some_and(|name| normalized_signal_name(name) == "KILL")
            || self.signal_number == Some(9);

        let mut result = ExecutionResult::success();
        let mut pids = Vec::new();
        for arg in &self.args {
            if arg.starts_with('%') {
                writeln!(
                    context.stderr(),
                    "{}: {}: job control signals are not supported on Windows",
                    context.command_name,
                    arg
                )?;
                result = ExecutionResult::general_error();
                continue;
            }

            let pid = crate::engine::int_utils::parse(arg.as_str(), 10)?;
            pids.push((arg.clone(), pid));
        }

        let statuses = compio::runtime::spawn_blocking(move || kill_pids(pids, force))
            .await
            .map_err(|err| crate::engine::ErrorKind::ThreadingError(err.to_string()))?;

        for (arg, succeeded) in statuses {
            if !succeeded {
                writeln!(
                    context.stderr(),
                    "{}: {}: failed to terminate process",
                    context.command_name,
                    arg
                )?;
                result = ExecutionResult::general_error();
            }
        }

        Ok(result)
    }
}

fn kill_pids(pids: Vec<(String, u32)>, force: bool) -> Vec<(String, bool)> {
    pids.into_iter()
        .map(|(arg, pid)| {
            let pid_arg = pid.to_string();
            let mut command = Command::new("taskkill");
            command.args(["/PID", pid_arg.as_str(), "/T"]);
            if force {
                command.arg("/F");
            }
            let succeeded = command.status().is_ok_and(|status| status.success());
            (arg, succeeded)
        })
        .collect()
}

fn print_signals(mut stdout: impl Write) -> Result<ExecutionResult, crate::engine::Error> {
    writeln!(stdout, " 1) SIGHUP")?;
    writeln!(stdout, " 2) SIGINT")?;
    writeln!(stdout, " 3) SIGQUIT")?;
    writeln!(stdout, " 9) SIGKILL")?;
    writeln!(stdout, "15) SIGTERM")?;
    Ok(ExecutionResult::success())
}

fn normalized_signal_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    upper.strip_prefix("SIG").unwrap_or(&upper).to_owned()
}

fn is_known_signal_name(name: &str) -> bool {
    matches!(
        normalized_signal_name(name).as_str(),
        "HUP" | "INT" | "QUIT" | "KILL" | "TERM"
    )
}

fn is_known_signal_number(number: usize) -> bool {
    matches!(number, 1 | 2 | 3 | 9 | 15)
}

fn is_supported_signal_name(name: &str) -> bool {
    matches!(normalized_signal_name(name).as_str(), "KILL" | "TERM")
}

fn is_supported_signal_number(number: usize) -> bool {
    matches!(number, 9 | 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_signal_shorthand_and_rejects_unimplemented_signals() {
        let command =
            <KillCommand as builtins::Command>::new(["kill", "-TERM", "123"].map(String::from))
                .unwrap();
        assert_eq!(command.signal_name.as_deref(), Some("TERM"));
        assert!(is_supported_signal_name("TERM"));
        assert!(is_supported_signal_name("SIGKILL"));
        assert!(!is_supported_signal_name("SIGINT"));

        let command =
            <KillCommand as builtins::Command>::new(["kill", "-9", "123"].map(String::from))
                .unwrap();
        assert_eq!(command.signal_number, Some(9));
    }
}
