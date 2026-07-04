use std::io::Write;

use crate::engine::commands::{self, ResolveOptions, ResolvedCommand};
use crate::engine::sys;
use crate::engine::{ExecutionResult, builtins};

/// Inspect the type of a named shell item.
pub(crate) struct TypeCommand {
    /// Display all locations of the specified name, not just the first.
    all_locations: bool,

    /// Don't consider functions when resolving the name.
    suppress_func_lookup: bool,

    /// Force searching by file path, even if the name is an alias, built-in
    /// command, or shell function.
    force_path_search: bool,

    /// Show file path only.
    show_path_only: bool,

    /// Only display the type of the specified name.
    type_only: bool,

    /// Names to search for.
    names: Vec<String>,
}

impl builtins::Command for TypeCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            all_locations: false,
            suppress_func_lookup: false,
            force_path_search: false,
            show_path_only: false,
            type_only: false,
            names: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);
        let names = args.parse_flags(|flag| match flag {
            'a' => {
                command.all_locations = true;
                Ok(true)
            }
            'f' => {
                command.suppress_func_lookup = true;
                Ok(true)
            }
            'P' => {
                command.force_path_search = true;
                Ok(true)
            }
            'p' => {
                command.show_path_only = true;
                Ok(true)
            }
            't' => {
                command.type_only = true;
                Ok(true)
            }
            _ => Err(format!("type: -{flag}: invalid option")),
        })?;
        command.names = names;
        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut result = ExecutionResult::success();

        for name in &self.names {
            let resolved_types = commands::resolve_command(
                context.shell,
                name,
                &ResolveOptions {
                    include_aliases: true,
                    include_keywords: true,
                    include_functions: !self.suppress_func_lookup,
                    include_builtins: true,
                    include_disabled_builtins: false,
                    include_path: true,
                    include_hashed: true,
                    all_locations: self.all_locations,
                    force_path_search: self.force_path_search,
                    use_default_path: false,
                    literal_path_with_separator: true,
                },
            );

            if resolved_types.is_empty() {
                if !self.type_only && !self.force_path_search && !self.show_path_only {
                    writeln!(context.stderr(), "type: {name} not found")?;
                }

                result = ExecutionResult::general_error();
                continue;
            }

            for resolved_type in resolved_types {
                if self.show_path_only && !matches!(resolved_type, ResolvedCommand::External { .. })
                {
                    // Do nothing.
                } else if self.type_only {
                    match resolved_type {
                        ResolvedCommand::Alias(_) => {
                            writeln!(context.stdout(), "alias")?;
                        }
                        ResolvedCommand::Keyword => {
                            writeln!(context.stdout(), "keyword")?;
                        }
                        ResolvedCommand::Function(_) => {
                            writeln!(context.stdout(), "function")?;
                        }
                        ResolvedCommand::Builtin => {
                            writeln!(context.stdout(), "builtin")?;
                        }
                        ResolvedCommand::External { path, .. } => {
                            if self.show_path_only || self.force_path_search {
                                writeln!(context.stdout(), "{}", sys::fs::display_path(&path))?;
                            } else {
                                writeln!(context.stdout(), "file")?;
                            }
                        }
                    }
                } else {
                    match resolved_type {
                        ResolvedCommand::Alias(target) => {
                            writeln!(context.stdout(), "{name} is aliased to `{target}'")?;
                        }
                        ResolvedCommand::Keyword => {
                            writeln!(context.stdout(), "{name} is a shell keyword")?;
                        }
                        ResolvedCommand::Function(def) => {
                            writeln!(context.stdout(), "{name} is a function")?;
                            writeln!(context.stdout(), "{def}")?;
                        }
                        ResolvedCommand::Builtin => {
                            writeln!(context.stdout(), "{name} is a shell builtin")?;
                        }
                        ResolvedCommand::External { path, hashed } => {
                            if hashed && self.all_locations && !self.force_path_search {
                                // Do nothing.
                            } else if self.show_path_only || self.force_path_search {
                                writeln!(context.stdout(), "{}", sys::fs::display_path(&path))?;
                            } else if hashed {
                                writeln!(
                                    context.stdout(),
                                    "{name} is hashed ({})",
                                    sys::fs::display_path(&path)
                                )?;
                            } else {
                                writeln!(
                                    context.stdout(),
                                    "{name} is {}",
                                    sys::fs::display_path(&path)
                                )?;
                            }
                        }
                    }
                }

                // If we only want the first, then break after the first.
                if !self.all_locations {
                    break;
                }
            }
        }

        Ok(result)
    }
}
