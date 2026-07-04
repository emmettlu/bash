use std::io::Write;

use crate::engine::{ExecutionResult, builtins};

/// Unset a shell alias.
pub(crate) struct UnaliasCommand {
    /// Remove all aliases.
    remove_all: bool,

    /// Names of aliases to operate on.
    aliases: Vec<String>,
}

impl builtins::Command for UnaliasCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            remove_all: false,
            aliases: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);
        let aliases = args.parse_flags(|flag| match flag {
            'a' => {
                command.remove_all = true;
                Ok(true)
            }
            _ => Err(format!("unalias: -{flag}: invalid option")),
        })?;
        command.aliases = aliases;
        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut exit_code = ExecutionResult::success();

        if self.remove_all {
            context.shell.aliases_mut().clear();
        } else {
            for alias in &self.aliases {
                if context.shell.aliases_mut().remove(alias).is_none() {
                    writeln!(
                        context.stderr(),
                        "{}: {}: not found",
                        context.command_name,
                        alias
                    )?;
                    exit_code = ExecutionResult::general_error();
                }
            }
        }

        Ok(exit_code)
    }
}
