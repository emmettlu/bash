use std::io::Write;

use crate::engine::{ExecutionControlFlow, ExecutionResult, builtins};

/// Return from the current function.
pub(crate) struct ReturnCommand {
    /// The exit code to return.
    code: Option<i32>,
}

impl builtins::Command for ReturnCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let code = match args.next_arg() {
            Some(arg) => Some(
                arg.parse()
                    .map_err(|_| format!("return: {arg}: numeric argument required"))?,
            ),
            None => None,
        };

        if args.next_arg().is_some() {
            return Err("return: too many arguments".into());
        }

        Ok(Self { code })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        #[expect(clippy::cast_sign_loss)]
        let code_8bit = if let Some(code_32bit) = &self.code {
            (code_32bit & 0xFF) as u8
        } else {
            context.shell.last_exit_status()
        };

        if context.shell.in_function() || context.shell.in_sourced_script() {
            let mut result = ExecutionResult::new(code_8bit);
            result.next_control_flow = ExecutionControlFlow::ReturnFromFunctionOrScript;

            Ok(result)
        } else {
            let _ = writeln!(
                context.stderr(),
                "return: can only be used in a function or sourced script"
            );
            Ok(ExecutionResult::invalid_usage())
        }
    }
}
