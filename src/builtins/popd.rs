use crate::engine::{ExecutionResult, builtins};

/// Pop a path from the current directory stack.
pub(crate) struct PopdCommand {
    /// Pop the path without changing the current working directory.
    no_directory_change: bool,
    //
    // TODO(popd): implement +N and -N
}

impl builtins::Command for PopdCommand {
    type Error = crate::builtins::dirs::DirError;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut no_directory_change = false;
        let mut args = builtins::BuiltinArgs::new(args);
        let positionals = args.parse_flags(|flag| match flag {
            'n' => {
                no_directory_change = true;
                Ok(true)
            }
            _ => Err(format!("-{flag}: invalid option")),
        })?;

        if !positionals.is_empty() {
            return Err(String::from("too many arguments"));
        }

        Ok(Self {
            no_directory_change,
        })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if let Some(popped) = context.shell.directory_stack_mut().pop() {
            if !self.no_directory_change {
                context.shell.set_working_dir(&popped)?;
            }

            // Display dirs.
            let dirs_cmd = crate::builtins::dirs::DirsCommand::default();
            dirs_cmd.execute(context).await?;

            Ok(ExecutionResult::success())
        } else {
            Err(crate::builtins::dirs::DirError::DirStackEmpty)
        }
    }
}
