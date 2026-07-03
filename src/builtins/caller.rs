use crate::engine::{ExecutionResult, builtins, callstack};
use clap::Parser;
use std::io::Write;

/// Return the context of the current subroutine call.
#[derive(Parser)]
pub(crate) struct CallerCommand {
    /// The number of call frames to go back.
    expr: Option<usize>,
}

impl builtins::Command for CallerCommand {
    type Error = crate::engine::Error;

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        let stack = context.shell.call_stack();

        // See how far back we need to look. Frame N represents the Nth caller
        // (e.g., 0 = immediate caller, 1 = caller's caller, etc.).
        let expr = self.expr.unwrap_or(0);

        let Some(target_index) = expr.checked_add(1) else {
            return Ok(ExecutionResult::general_error());
        };

        // Look for the last-known location in the parent of frame N.
        let Some(calling_frame) = stack
            .iter()
            .filter(|frame| frame.frame_type.is_function() || frame.frame_type.is_script())
            .nth(target_index)
        else {
            return Ok(ExecutionResult::general_error());
        };

        let line = calling_frame.current_line().unwrap_or(1);
        let filename = &calling_frame.source_info.source;

        // When the expr is provided, we display "LINE FUNCTION_NAME FILENAME"
        // When the expr is omitted, we only display "LINE FILENAME"
        if self.expr.is_some() {
            let function_name = match &calling_frame.frame_type {
                callstack::FrameType::Function(func_call) => func_call.name(),
                callstack::FrameType::Script(..) => "source".into(),
                _ => "".into(),
            };

            writeln!(context.stdout(), "{line} {function_name} {filename}")?;
        } else {
            writeln!(context.stdout(), "{line} {filename}")?;
        }

        Ok(ExecutionResult::success())
    }
}
