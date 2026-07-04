use crate::engine::{ExecutionControlFlow, ExecutionResult, builtins};

/// Breaks out of a control-flow loop.
pub(crate) struct BreakCommand {
    /// If specified, indicates which nested loop to break out of.
    which_loop: i8,
}

impl builtins::Command for BreakCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let args = builtins::BuiltinArgs::new(args).rest();
        match args.as_slice() {
            [] => Ok(Self { which_loop: 1 }),
            [which_loop] => Ok(Self {
                which_loop: which_loop
                    .parse()
                    .map_err(|_| format!("break: {which_loop}: numeric argument required"))?,
            }),
            _ => Err("break: too many arguments".into()),
        }
    }

    async fn execute(
        &self,
        _context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        // If specified, which_loop needs to be positive.
        if self.which_loop <= 0 {
            return Ok(ExecutionResult::invalid_usage());
        }

        let mut result = ExecutionResult::success();

        result.next_control_flow = ExecutionControlFlow::BreakLoop {
            #[expect(clippy::cast_sign_loss)]
            levels: (self.which_loop - 1) as usize,
        };

        Ok(result)
    }
}
