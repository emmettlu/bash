use crate::engine::{ExecutionResult, builtins};

/// Push a path onto the current directory stack.
pub(crate) struct PushdCommand {
    /// Push the path without changing the current working directory.
    no_directory_change: bool,

    /// Directory to push on the directory stack.
    dir: String,
    //
    // TODO(pushd): implement +N and -N
}

impl builtins::Command for PushdCommand {
    type Error = crate::engine::Error;

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

        let [dir] = positionals.as_slice() else {
            return Err(if positionals.is_empty() {
                String::from("missing directory")
            } else {
                String::from("too many arguments")
            });
        };

        Ok(Self {
            no_directory_change,
            dir: dir.clone(),
        })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.no_directory_change {
            context
                .shell
                .directory_stack_mut()
                .push(std::path::PathBuf::from(&self.dir));
        } else {
            let prev_working_dir = context.shell.working_dir().to_path_buf();

            let dir = std::path::Path::new(&self.dir);
            context.shell.set_working_dir(dir)?;

            context.shell.directory_stack_mut().push(prev_working_dir);
        }

        // Display dirs.
        let dirs_cmd = crate::builtins::dirs::DirsCommand::default();
        dirs_cmd.execute(context).await?;

        Ok(ExecutionResult::success())
    }
}
