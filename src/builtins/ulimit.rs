use clap::Parser;
use std::io::Write;

use crate::core::{ExecutionExitCode, ExecutionResult, builtins};

/// Control shell resource limits.
#[derive(Parser)]
pub(crate) struct ULimitCommand {
    /// Show all current limits.
    #[arg(short = 'a')]
    all: bool,

    /// Set soft limit.
    #[arg(short = 'S')]
    soft: bool,

    /// Set hard limit.
    #[arg(short = 'H')]
    hard: bool,

    /// Limit value.
    value: Option<String>,
}

impl builtins::Command for ULimitCommand {
    type Error = crate::core::Error;

    async fn execute<SE: crate::core::ShellExtensions>(
        &self,
        context: crate::core::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        let _ = (self.soft, self.hard);

        if self.value.is_some() {
            writeln!(
                context.stderr(),
                "{}: setting resource limits is not supported on Windows",
                context.command_name
            )?;
            return Ok(ExecutionExitCode::CannotExecute.into());
        }

        if self.all {
            writeln!(
                context.stdout(),
                "core file size          (blocks, -c) unlimited"
            )?;
            writeln!(
                context.stdout(),
                "data seg size           (kbytes, -d) unlimited"
            )?;
            writeln!(
                context.stdout(),
                "file size               (blocks, -f) unlimited"
            )?;
            writeln!(
                context.stdout(),
                "open files                      (-n) unlimited"
            )?;
            writeln!(
                context.stdout(),
                "stack size              (kbytes, -s) unlimited"
            )?;
        } else {
            writeln!(context.stdout(), "unlimited")?;
        }

        Ok(ExecutionResult::success())
    }
}
