use crate::engine::sys;
use std::{io::Write, path::PathBuf};

use crate::engine::{ExecutionResult, builtins};

pub(crate) struct HashCommand {
    /// Remove entries associated with the given names.
    remove: bool,

    /// Display paths in a format usable for input.
    display_as_usable_input: bool,

    /// The path to associate with the names.
    path_to_use: Option<PathBuf>,

    /// Remove all entries.
    remove_all: bool,

    /// Display the paths associated with the names.
    display_paths: bool,

    /// Names to process.
    names: Vec<String>,
}

impl builtins::Command for HashCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            remove: false,
            display_as_usable_input: false,
            path_to_use: None,
            remove_all: false,
            display_paths: false,
            names: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);

        while let Some(arg) = args.next_arg() {
            if arg == "--" {
                command.names.extend(args.rest());
                break;
            }

            let Some(flags) = arg.strip_prefix('-') else {
                command.names.push(arg);
                command.names.extend(args.rest());
                break;
            };

            if flags.is_empty() {
                command.names.push(arg);
                command.names.extend(args.rest());
                break;
            }

            for (idx, flag) in flags.char_indices() {
                match flag {
                    'd' => command.remove = true,
                    'l' => command.display_as_usable_input = true,
                    'r' => command.remove_all = true,
                    't' => command.display_paths = true,
                    'p' => {
                        let value_start = idx + flag.len_utf8();
                        let value = if value_start < flags.len() {
                            flags[value_start..].to_owned()
                        } else {
                            args.next_value("-p")?
                        };
                        command.path_to_use = Some(PathBuf::from(value));
                        break;
                    }
                    _ => return Err(format!("-{flag}: invalid option")),
                }
            }
        }

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut result = ExecutionResult::success();

        if self.remove_all {
            context.shell.program_location_cache_mut().reset();
        } else if self.remove {
            for name in &self.names {
                if !context.shell.program_location_cache_mut().unset(name) {
                    writeln!(context.stderr(), "{name}: not found")?;
                    result = ExecutionResult::general_error();
                }
            }
        } else if self.display_paths {
            for name in &self.names {
                if let Some(path) = context.shell.program_location_cache().get(name) {
                    if self.display_as_usable_input {
                        writeln!(
                            context.stdout(),
                            "builtin hash -p {} {name}",
                            sys::fs::display_path(path.as_path())
                        )?;
                    } else {
                        let mut prefix = String::new();

                        if self.names.len() > 1 {
                            prefix.push_str(name.as_str());
                            prefix.push('\t');
                        }

                        writeln!(
                            context.stdout(),
                            "{prefix}{}",
                            sys::fs::display_path(path.as_path())
                        )?;
                    }
                } else {
                    writeln!(context.stderr(), "{name}: not found")?;
                    result = ExecutionResult::general_error();
                }
            }
        } else if let Some(path) = &self.path_to_use {
            for name in &self.names {
                context
                    .shell
                    .program_location_cache_mut()
                    .set(name, path.clone());
            }
        } else {
            for name in &self.names {
                // Remove from the cache if already hashed.
                let _ = context.shell.program_location_cache_mut().unset(name);

                // Names with slashes are accepted silently
                if name.contains('/') {
                    continue;
                }

                // Hash the path
                if context
                    .shell
                    .find_first_executable_in_path_using_cache(name)
                    .is_none()
                {
                    writeln!(context.stderr(), "{name}: not found")?;
                    result = ExecutionResult::general_error();
                }
            }
        }

        Ok(result)
    }
}
