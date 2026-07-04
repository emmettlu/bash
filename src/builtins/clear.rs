use std::io::Write;

use crate::engine::{ExecutionResult, builtins};

/// Clear the terminal screen.
pub(crate) struct ClearCommand {}

impl builtins::Command for ClearCommand {
    type Error = crate::engine::Error;

    fn new<I>(_args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {})
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        write!(context.stdout(), "\x1B[2J\x1B[H")?;
        context.stdout().flush()?;
        Ok(ExecutionResult::success())
    }
}
