use std::path::Path;

use crate::engine::builtins;

/// Evaluate the provided script in the current shell environment.
pub(crate) struct DotCommand {
    /// Path to the script to evaluate.
    script_path: String,

    /// Any arguments to be passed as positional parameters to the script.
    script_args: Vec<String>,
}

impl builtins::Command for DotCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args).rest();
        if args.first().is_some_and(|arg| arg == "--") {
            args.remove(0);
        }

        if args.is_empty() {
            return Err(".: filename argument required".into());
        }

        let script_path = args.remove(0);
        Ok(Self {
            script_path,
            script_args: args,
        })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        // TODO(dot): Handle trap inheritance.
        context
            .shell
            .source_script(
                Path::new(&self.script_path),
                self.script_args.iter(),
                &context.params,
            )
            .await
    }
}
