use clap::Parser;
use std::io::Write;

use crate::core::{ExecutionResult, builtins};

/// Clear the terminal screen.
#[derive(Parser)]
pub(crate) struct ClearCommand {}

impl builtins::Command for ClearCommand {
    type Error = crate::core::Error;

    async fn execute<SE: crate::core::ShellExtensions>(
        &self,
        context: crate::core::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        write!(context.stdout(), "\x1B[2J\x1B[H")?;
        context.stdout().flush()?;
        Ok(ExecutionResult::success())
    }
}
