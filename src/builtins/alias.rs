use std::io::Write;

use crate::engine::{ExecutionResult, builtins};

/// Manage aliases within the shell.
pub(crate) struct AliasCommand {
    /// Print all defined aliases in a reusable format.
    print: bool,

    /// List of aliases to display or update.
    aliases: Vec<String>,
}

impl builtins::Command for AliasCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let mut print = false;
        let aliases = args.parse_flags(|flag| match flag {
            'p' => {
                print = true;
                Ok(true)
            }
            _ => Err(format!("alias: -{flag}: invalid option")),
        })?;

        Ok(Self { print, aliases })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut exit_code = ExecutionResult::success();

        if self.print || self.aliases.is_empty() {
            for (name, value) in context.shell.aliases() {
                writeln!(context.stdout(), "alias {name}='{value}'")?;
            }
        } else {
            for alias in &self.aliases {
                if let Some((name, unexpanded_value)) = alias.split_once('=')
                    && !name.is_empty()
                {
                    context
                        .shell
                        .aliases_mut()
                        .insert(name.to_owned(), unexpanded_value.to_owned());
                } else if let Some(value) = context.shell.aliases().get(alias) {
                    writeln!(context.stdout(), "alias {alias}='{value}'")?;
                } else {
                    writeln!(
                        context.stderr(),
                        "{}: {alias}: not found",
                        context.command_name
                    )?;
                    exit_code = ExecutionResult::general_error();
                }
            }
        }

        Ok(exit_code)
    }
}
