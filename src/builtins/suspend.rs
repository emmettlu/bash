use clap::Parser;
use std::io::Write;

use crate::core::{ExecutionResult, builtins};

/// Suspend shell execution.
#[derive(Parser)]
pub(crate) struct SuspendCommand {
    /// Do not complain if this is a login shell.
    #[arg(short = 'f')]
    force: bool,
}

impl builtins::Command for SuspendCommand {
    type Error = crate::core::Error;

    async fn execute<SE: crate::core::ShellExtensions>(
        &self,
        context: crate::core::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        let _ = self.force;
        writeln!(
            context.stderr(),
            "{}: suspend is not supported on Windows",
            context.command_name
        )?;
        Ok(ExecutionResult::general_error())
    }
}
