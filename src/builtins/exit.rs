use crate::engine::{ExecutionControlFlow, ExecutionResult, builtins};

/// Exit the shell.
pub(crate) struct ExitCommand {
    /// The exit code to return.
    code: Option<i64>,
}

impl builtins::Command for ExitCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args).rest();
        if args.first().is_some_and(|arg| arg == "--") {
            args.remove(0);
        }

        match args.as_slice() {
            [] => Ok(Self { code: None }),
            [code] => Ok(Self {
                code: Some(
                    code.parse()
                        .map_err(|_| format!("exit: {code}: numeric argument required"))?,
                ),
            }),
            _ => Err("exit: too many arguments".into()),
        }
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

        let mut result = ExecutionResult::new(code_8bit);
        result.next_control_flow = ExecutionControlFlow::ExitShell;

        Ok(result)
    }
}
