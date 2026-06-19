use clap::Parser;
use std::{fmt::Display, io::Write, path::Path};

use crate::engine::{
    ExecutionResult, builtins, commands, pathsearch,
    sys::{self, traits::PathExt},
};

/// Directly invokes an external command, without going through typical search order.
#[derive(Default, Parser)]
pub(crate) struct CommandCommand {
    /// Use default PATH value.
    #[arg(short = 'p')]
    pub use_default_path: bool,

    /// Display a short description of the command.
    #[arg(short = 'v')]
    pub print_description: bool,

    /// Display a more verbose description of the command.
    #[arg(short = 'V')]
    pub print_verbose_description: bool,

    /// Command and arguments.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command_and_args: Vec<String>,
}

impl CommandCommand {
    fn command(&self) -> Option<&str> {
        self.command_and_args.first().map(String::as_str)
    }
}

impl builtins::Command for CommandCommand {
    type Error = crate::engine::Error;

    async fn execute<SE: crate::engine::ShellExtensions>(
        &self,
        context: crate::engine::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        // Silently exit if no command was provided.
        if let Some(command_name) = self.command() {
            if self.print_description || self.print_verbose_description {
                if let Some(found_cmd) =
                    Self::try_find_command(context.shell, command_name, self.use_default_path)
                {
                    if self.print_description {
                        writeln!(context.stdout(), "{found_cmd}")?;
                    } else {
                        match found_cmd {
                            FoundCommand::Builtin(_name) => {
                                writeln!(context.stdout(), "{command_name} is a shell builtin")?;
                            }
                            FoundCommand::External(path) => {
                                writeln!(context.stdout(), "{command_name} is {path}")?;
                            }
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

enum FoundCommand<'a> {
    Builtin(&'a str),
    External(String),
}

impl Display for FoundCommand<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin(name) => write!(f, "{name}"),
            Self::External(path) => write!(f, "{path}"),
        }
    }
}

impl CommandCommand {
    fn try_find_command<'a>(
        shell: &mut crate::engine::Shell<impl crate::engine::ShellExtensions>,
        command_name: &'a str,
        use_default_path: bool,
    ) -> Option<FoundCommand<'a>> {
        // Look in path.
        if sys::fs::contains_path_separator(command_name) {
            let candidate_path = shell.absolute_path(Path::new(command_name));
            if candidate_path.executable() {
                Some(FoundCommand::External(sys::fs::display_path(
                    &candidate_path,
                )))
            } else {
                None
            }
        } else {
            if let Some(builtin_cmd) = shell.builtins().get(command_name)
                && !builtin_cmd.disabled
            {
                return Some(FoundCommand::Builtin(command_name));
            }

            if use_default_path {
                let dirs = sys::fs::get_default_standard_utils_paths();

                pathsearch::search_for_executable(dirs.iter(), command_name)
                    .next()
                    .map(|path| FoundCommand::External(sys::fs::display_path(&path)))
            } else {
                shell
                    .find_first_executable_in_path_using_cache(command_name)
                    .map(|path| FoundCommand::External(sys::fs::display_path(&path)))
            }
        }
    }

    async fn execute_command(
        &self,
        mut context: crate::engine::ExecutionContext<'_, impl crate::engine::ShellExtensions>,
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
            commands::ShellForCommand::ParentShell(context.shell),
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
