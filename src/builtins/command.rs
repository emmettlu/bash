use std::{io::Write, path::PathBuf};

use crate::engine::{ExecutionResult, builtins, commands, sys};

/// Directly invokes an external command, without going through typical search order.
#[derive(Default)]
pub(crate) struct CommandCommand {
    /// Use default PATH value.
    pub use_default_path: bool,

    /// Display a short description of the command.
    pub print_description: bool,

    /// Display a more verbose description of the command.
    pub print_verbose_description: bool,

    /// Command and arguments.
    pub command_and_args: Vec<String>,
}

impl CommandCommand {
    fn command(&self) -> Option<&str> {
        self.command_and_args.first().map(String::as_str)
    }
}

impl builtins::Command for CommandCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let mut command = Self::default();
        command.command_and_args = args.parse_flags(|flag| match flag {
            'p' => {
                command.use_default_path = true;
                Ok(true)
            }
            'v' => {
                command.print_description = true;
                Ok(true)
            }
            'V' => {
                command.print_verbose_description = true;
                Ok(true)
            }
            _ => Err(format!("command: -{flag}: invalid option")),
        })?;
        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        // Silently exit if no command was provided.
        if let Some(command_name) = self.command() {
            if self.print_description || self.print_verbose_description {
                if let Some(found_cmd) =
                    Self::try_find_command(context.shell, command_name, self.use_default_path)
                {
                    match (self.print_description, found_cmd) {
                        (true, FoundCommand::Builtin) => {
                            writeln!(context.stdout(), "{command_name}")?;
                        }
                        (true, FoundCommand::External(path)) => {
                            writeln!(context.stdout(), "{}", sys::fs::display_path(&path))?;
                        }
                        (false, FoundCommand::Builtin) => {
                            writeln!(context.stdout(), "{command_name} is a shell builtin")?;
                        }
                        (false, FoundCommand::External(path)) => {
                            writeln!(
                                context.stdout(),
                                "{command_name} is {}",
                                sys::fs::display_path(&path)
                            )?;
                        }
                    }
                    Ok(ExecutionResult::success())
                } else {
                    if self.print_verbose_description {
                        writeln!(context.stderr(), "command: {command_name}: not found")?;
                    }
                    Ok(ExecutionResult::general_error())
                }
            } else {
                self.execute_command(context, command_name, self.use_default_path)
                    .await
            }
        } else {
            Ok(ExecutionResult::success())
        }
    }
}

enum FoundCommand {
    Builtin,
    External(PathBuf),
}

impl CommandCommand {
    fn try_find_command(
        shell: &crate::engine::Shell,
        command_name: &str,
        use_default_path: bool,
    ) -> Option<FoundCommand> {
        commands::resolve_command(
            shell,
            command_name,
            &commands::ResolveOptions {
                include_aliases: false,
                include_keywords: false,
                include_functions: false,
                include_builtins: true,
                include_disabled_builtins: false,
                include_path: true,
                include_hashed: !use_default_path,
                all_locations: false,
                force_path_search: false,
                use_default_path,
                literal_path_with_separator: false,
            },
        )
        .into_iter()
        .find_map(|resolved| match resolved {
            commands::ResolvedCommand::Builtin => Some(FoundCommand::Builtin),
            commands::ResolvedCommand::External { path, .. } => Some(FoundCommand::External(path)),
            commands::ResolvedCommand::Alias(_)
            | commands::ResolvedCommand::Keyword
            | commands::ResolvedCommand::Function(_) => None,
        })
    }

    async fn execute_command(
        &self,
        mut context: crate::engine::ExecutionContext<'_>,
        command_name: &str,
        use_default_path: bool,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        command_name.clone_into(&mut context.command_name);
        let command_and_args = self
            .command_and_args
            .iter()
            .map(|arg| commands::CommandArg::from(arg.as_str()))
            .collect();

        let path_dirs = if use_default_path {
            Some(sys::fs::get_default_standard_utils_paths())
        } else {
            None
        };

        let mut cmd = commands::SimpleCommand::new(
            commands::ShellForCommand::parent(context.shell),
            context.params,
            context.command_name,
            command_and_args,
        );
        cmd.use_functions = false;
        cmd.path_dirs = path_dirs;

        let spawn_result = cmd.execute().await?;
        let wait_result = spawn_result.wait().await?;

        Ok(wait_result.into())
    }
}
