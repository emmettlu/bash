use crate::engine::{ExecutionResult, builtins};

/// Shift positional arguments.
pub(crate) struct ShiftCommand {
    /// Number of positions to shift the arguments by (defaults to 1).
    n: Option<i32>,
}

impl builtins::Command for ShiftCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let n = match args.next_arg() {
            Some(arg) => Some(
                arg.parse()
                    .map_err(|_| format!("shift: {arg}: numeric argument required"))?,
            ),
            None => None,
        };

        if args.next_arg().is_some() {
            return Err("shift: too many arguments".into());
        }

        Ok(Self { n })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let n = self.n.unwrap_or(1);

        if n < 0 {
            return Ok(ExecutionResult::invalid_usage());
        }

        #[expect(clippy::cast_sign_loss)]
        let n = n as usize;

        let args = context.shell.current_shell_args_mut();

        if n > args.len() {
            return Ok(ExecutionResult::invalid_usage());
        }

        args.drain(0..n);

        Ok(ExecutionResult::success())
    }
}
