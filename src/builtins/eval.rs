use crate::engine::{ExecutionResult, builtins};

/// Evaluate the given string as script.
pub(crate) struct EvalCommand {
    /// The script to evaluate.
    args: Vec<String>,
}

impl builtins::Command for EvalCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {
            args: builtins::BuiltinArgs::new(args).rest(),
        })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if !self.args.is_empty() {
            let args_concatenated = self.args.join(" ");

            log::debug!("Applying eval to: {:?}", args_concatenated);

            // Our new source context is relative to the current position because we are only
            // providing the raw string being eval'd.
            // TODO(source-info): Provide the location of the specific tokens that make up
            // `self.args`.
            let source_info = context.shell.call_stack().current_pos_as_source_info();

            // Return the direct result of running the string; we intentionally
            // pass through the result and honor its requested control flow. eval
            // executes in the current environment, so all control flow (return,
            // exit, break, continue) should propagate.
            context
                .shell
                .run_string(args_concatenated, &source_info, &context.params)
                .await
        } else {
            Ok(ExecutionResult::success())
        }
    }
}
