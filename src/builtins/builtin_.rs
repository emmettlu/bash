use crate::engine::{CommandArg, ExecutionResult, builtins};

/// Directly invokes a built-in, without going through typical search order.
#[derive(Default)]
pub(crate) struct BuiltinCommand {
    args: Vec<CommandArg>,
}

impl builtins::DeclarationCommand for BuiltinCommand {
    fn set_declarations(&mut self, args: Vec<CommandArg>) {
        self.args = args;
    }
}

impl builtins::Command for BuiltinCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {
            args: args.into_iter().map(CommandArg::String).collect(),
        })
    }

    async fn execute(
        &self,
        mut context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.args.is_empty() {
            return Ok(ExecutionResult::success());
        }

        let args: Vec<_> = self.args.iter().skip(1).cloned().collect();
        if args.is_empty() {
            return Ok(ExecutionResult::success());
        }

        let builtin_name = args[0].to_string();

        if let Some(builtin) = context.shell.builtins().get(&builtin_name)
            && !builtin.disabled
        {
            context.command_name = builtin_name;
            (builtin.execute_func)(context, args).await
        } else {
            Err(crate::engine::ErrorKind::BuiltinNotFound(builtin_name).into())
        }
    }
}
