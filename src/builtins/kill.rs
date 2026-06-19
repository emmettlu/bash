use clap::Parser;
use std::io::Write;
use std::process::Command;

use crate::engine::{ExecutionExitCode, ExecutionResult, builtins};

/// Signal a job or process.
#[derive(Parser)]
pub(crate) struct KillCommand {
    /// Name of the signal to send.
    #[arg(short = 's', value_name = "SIG_NAME")]
    signal_name: Option<String>,

    /// Number of the signal to send.
    #[arg(short = 'n', value_name = "SIG_NUM")]
    signal_number: Option<usize>,

    /// List signal names.
    #[arg(short = 'l')]
    list_signals: bool,

    /// PIDs to terminate.
    args: Vec<String>,
}

impl builtins::Command for KillCommand {
    type Error = crate::engine::Error;

    async fn execute<SE: crate::engine::ShellExtensions>(
        &self,
        context: crate::engine::ExecutionContext<'_, SE>,
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
            return Ok(ExecutionExitCode::InvalidUsage.into());
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
            return Ok(ExecutionExitCode::InvalidUsage.into());
        }

        if self.args.is_empty() {
            writeln!(context.stderr(), "{}: invalid usage", context.command_name)?;
            return Ok(ExecutionExitCode::InvalidUsage.into());
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
