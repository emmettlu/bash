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

        if let Some(signal_name) = &self.signal_name
            && !is_supported_signal_name(signal_name)
        {
            writeln!(
                context.stderr(),
                "{}: invalid signal name: {}",
                context.command_name,
                signal_name
            )?;
            return Ok(ExecutionResult::invalid_usage());
        }

        if let Some(signal_number) = self.signal_number
            && !is_supported_signal_number(signal_number)
        {
            writeln!(
                context.stderr(),
                "{}: invalid signal number: {}",
                context.command_name,
                signal_number
            )?;
            return Ok(ExecutionResult::invalid_usage());
        }

        if self.args.is_empty() {
            writeln!(context.stderr(), "{}: invalid usage", context.command_name)?;
            return Ok(ExecutionResult::invalid_usage());
        }

        let mut result = ExecutionResult::success();
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
            if !kill_pid(pid) {
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

fn kill_pid(pid: u32) -> bool {
    let pid_arg = pid.to_string();
    Command::new("taskkill")
        .args(["/PID", pid_arg.as_str(), "/T", "/F"])
        .status()
        .is_ok_and(|status| status.success())
}

fn print_signals(mut stdout: impl Write) -> Result<ExecutionResult, crate::engine::Error> {
    writeln!(stdout, " 1) SIGHUP")?;
    writeln!(stdout, " 2) SIGINT")?;
    writeln!(stdout, " 3) SIGQUIT")?;
    writeln!(stdout, " 9) SIGKILL")?;
    writeln!(stdout, "15) SIGTERM")?;
    Ok(ExecutionResult::success())
}

fn is_supported_signal_name(name: &str) -> bool {
    let name = name.trim_start_matches("SIG").to_ascii_uppercase();
    matches!(name.as_str(), "HUP" | "INT" | "QUIT" | "KILL" | "TERM")
}

fn is_supported_signal_number(number: usize) -> bool {
    matches!(number, 1 | 2 | 3 | 9 | 15)
}
