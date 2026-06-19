use clap::Parser;
use std::io::Write;

use crate::engine::{ExecutionResult, builtins};

/// Clear the terminal screen.
#[derive(Parser)]
pub(crate) struct ClearCommand {}

impl builtins::Command for ClearCommand {
    type Error = crate::engine::Error;

    async fn execute<SE: crate::engine::ShellExtensions>(
        &self,
        context: crate::engine::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        write!(context.stdout(), "\x1B[2J\x1B[H")?;
        context.stdout().flush()?;
        Ok(ExecutionResult::success())
    }
}
