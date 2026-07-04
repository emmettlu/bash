use crate::engine::ExecutionResult;
use itertools::Itertools;
use std::io::Write;

use crate::engine::builtins;
use crate::engine::error;

/// Enable, disable, or display built-in commands.
pub(crate) struct EnableCommand {
    /// Print a list of built-in commands.
    print_list: bool,

    /// Disables the specified built-in commands.
    disable: bool,

    /// Only operate on special built-in commands.
    special_only: bool,

    /// Path to a shared object from which built-in commands will be loaded.
    shared_object_path: Option<String>,

    /// Remove the built-in commands loaded from the indicated object path.
    remove_loaded_builtin: bool,

    /// Names of built-in commands to operate on.
    names: Vec<String>,
}

impl builtins::Command for EnableCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let mut command = Self {
            print_list: false,
            disable: false,
            special_only: false,
            shared_object_path: None,
            remove_loaded_builtin: false,
            names: Vec::new(),
        };

        command.names =
            args.parse_short_options_permuted(|args, flag, flags, rest_start| match flag {
                'a' => {
                    command.print_list = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                'n' => {
                    command.disable = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                'p' => Ok(builtins::ShortOptionDisposition::Continue),
                's' => {
                    command.special_only = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                'd' => {
                    command.remove_loaded_builtin = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                'f' => {
                    command.shared_object_path =
                        Some(args.option_value(flags, rest_start, "enable: -f")?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                _ => Err(format!("enable: -{flag}: invalid option")),
            })?;

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        let mut result = ExecutionResult::success();

        if self.shared_object_path.is_some() {
            return error::unimp("enable -f");
        }
        if self.remove_loaded_builtin {
            return error::unimp("enable -d");
        }

        if !self.names.is_empty() {
            for name in &self.names {
                if let Some(builtin) = context.shell.builtin_mut(name) {
                    builtin.disabled = self.disable;
                } else {
                    writeln!(context.stderr(), "{name}: not a shell builtin")?;
                    result = ExecutionResult::general_error();
                }
            }
        } else {
            let builtins: Vec<_> = context
                .shell
                .builtins()
                .iter()
                .sorted_by_key(|(name, _reg)| *name)
                .collect();

            for (builtin_name, builtin) in builtins {
                if self.disable {
                    if !builtin.disabled {
                        continue;
                    }
                } else if self.print_list && builtin.disabled {
                    continue;
                }

                if self.special_only && !builtin.special_builtin {
                    continue;
                }

                let prefix = if builtin.disabled { "-n " } else { "" };

                writeln!(context.stdout(), "enable {prefix}{builtin_name}")?;
            }
        }

        Ok(result)
    }
}
