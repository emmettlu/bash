use crate::engine::{ExecutionControlFlow, ExecutionResult, builtins};

/// Continue to the next iteration of a control-flow loop.
pub(crate) struct ContinueCommand {
    /// If specified, indicates which nested loop to continue to the next iteration of.
    which_loop: i8,
}

impl builtins::Command for ContinueCommand {
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
                    .map_err(|_| format!("continue: {which_loop}: numeric argument required"))?,
            }),
            _ => Err("continue: too many arguments".into()),
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

        result.next_control_flow = ExecutionControlFlow::ContinueLoop {
            #[expect(clippy::cast_sign_loss)]
            levels: (self.which_loop - 1) as usize,
        };

        Ok(result)
    }
}
