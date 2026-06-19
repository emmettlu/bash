use clap::Parser;

use crate::engine::{ExecutionExitCode, ExecutionResult, builtins};

/// Shift positional arguments.
#[derive(Parser)]
pub(crate) struct ShiftCommand {
    /// Number of positions to shift the arguments by (defaults to 1).
    n: Option<i32>,
}

impl builtins::Command for ShiftCommand {
    type Error = crate::engine::Error;

    async fn execute<SE: crate::engine::ShellExtensions>(
        &self,
        context: crate::engine::ExecutionContext<'_, SE>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let n = self.n.unwrap_or(1);

        if n < 0 {
            return Ok(ExecutionExitCode::InvalidUsage.into());
        }

        #[expect(clippy::cast_sign_loss)]
        let n = n as usize;

        let args = context.shell.current_shell_args_mut();

        if n > args.len() {
            return Ok(ExecutionExitCode::InvalidUsage.into());
        }

        args.drain(0..n);

        Ok(ExecutionResult::success())
    }
}
