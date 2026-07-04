use itertools::Itertools;
use std::io::Write;

use crate::engine::{ExecutionResult, builtins};

/// Manage shopt-style options.
pub(crate) struct ShoptCommand {
    /// Manage set -o options.
    set_o_names_only: bool,

    /// Print options' current values.
    print: bool,

    /// Suppress typical output.
    quiet: bool,

    /// Set the specified options.
    set: bool,

    /// Unset the specified options.
    unset: bool,

    /// Names of options to operate on.
    options: Vec<String>,
}

impl builtins::Command for ShoptCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            set_o_names_only: false,
            print: false,
            quiet: false,
            set: false,
            unset: false,
            options: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);
        let options = args.parse_flags(|flag| match flag {
            'o' => {
                command.set_o_names_only = true;
                Ok(true)
            }
            'p' => {
                command.print = true;
                Ok(true)
            }
            'q' => {
                command.quiet = true;
                Ok(true)
            }
            's' => {
                command.set = true;
                Ok(true)
            }
            'u' => {
                command.unset = true;
                Ok(true)
            }
            _ => Err(format!("shopt: -{flag}: invalid option")),
        })?;
        command.options = options;
        Ok(command)
    }

    #[allow(clippy::too_many_lines)]
    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.set && self.unset {
            writeln!(
                context.stderr(),
                "cannot set and unset shell options simultaneously"
            )?;
            return Ok(ExecutionResult::invalid_usage());
        }

        if self.options.is_empty() {
            if self.quiet {
                return Ok(ExecutionResult::success());
            }

            // Enumerate all options of the selected type.
            let options = if self.set_o_names_only {
                crate::engine::namedoptions::options(
                    crate::engine::namedoptions::ShellOptionKind::SetO,
                )
                .iter()
                .sorted_by_key(|opt| opt.name)
            } else {
                crate::engine::namedoptions::options(
                    crate::engine::namedoptions::ShellOptionKind::Shopt,
                )
                .iter()
                .sorted_by_key(|opt| opt.name)
            };

            for option in options {
                let option_value = option.definition.get(context.shell.options());
                if self.set && !option_value {
                    continue;
                }
                if self.unset && option_value {
                    continue;
                }

                if self.print {
                    if self.set_o_names_only {
                        let option_value_str = if option_value { "-o" } else { "+o" };
                        writeln!(context.stdout(), "set {option_value_str} {}", option.name)?;
                    } else {
                        let option_value_str = if option_value { "-s" } else { "-u" };
                        writeln!(context.stdout(), "shopt {option_value_str} {}", option.name)?;
                    }
                } else {
                    let option_value_str = if option_value { "on" } else { "off" };
                    writeln!(context.stdout(), "{:20}\t{option_value_str}", option.name)?;
                }
            }

            Ok(ExecutionResult::success())
        } else {
            let mut return_value = ExecutionResult::success();

            // Enumerate only the specified options.
            for option_name in &self.options {
                if option_name == "posix" {
                    continue;
                }

                let option_definition = if self.set_o_names_only {
                    crate::engine::namedoptions::options(
                        crate::engine::namedoptions::ShellOptionKind::SetO,
                    )
                    .get(option_name.as_str())
                } else {
                    crate::engine::namedoptions::options(
                        crate::engine::namedoptions::ShellOptionKind::Shopt,
                    )
                    .get(option_name.as_str())
                };

                if let Some(option_definition) = option_definition {
                    if self.set {
                        option_definition.set(context.shell.options_mut(), true);
                    } else if self.unset {
                        option_definition.set(context.shell.options_mut(), false);
                    } else {
                        let option_value = option_definition.get(context.shell.options());
                        if !option_value {
                            return_value = ExecutionResult::general_error();
                        }

                        if !self.quiet {
                            if self.print {
                                if self.set_o_names_only {
                                    let option_value_str = if option_value { "-o" } else { "+o" };
                                    writeln!(
                                        context.stdout(),
                                        "set {option_value_str} {option_name}"
                                    )?;
                                } else {
                                    let option_value_str = if option_value { "-s" } else { "-u" };
                                    writeln!(
                                        context.stdout(),
                                        "shopt {option_value_str} {option_name}"
                                    )?;
                                }
                            } else {
                                let option_value_str = if option_value { "on" } else { "off" };
                                writeln!(context.stdout(), "{option_name:20}\t{option_value_str}")?;
                            }
                        }
                    }
                } else {
                    writeln!(
                        context.stderr(),
                        "{}: {}: invalid shell option name",
                        context.command_name,
                        option_name
                    )?;
                    return_value = ExecutionResult::general_error();
                }
            }

            Ok(return_value)
        }
    }
}
