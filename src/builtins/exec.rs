use std::borrow::Cow;

use crate::engine::{ExecutionResult, builtins, commands};

/// Exec the provided command.
pub(crate) struct ExecCommand {
    /// Pass given name as zeroth argument to command.
    name_for_argv0: Option<String>,

    /// Exec command with an empty environment.
    empty_environment: bool,

    /// Exec command as a login shell.
    exec_as_login: bool,

    /// Command and args.
    args: Vec<String>,
}

impl builtins::Command for ExecCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let parsed_args = builtins::BuiltinArgs::new(args).rest();
        let mut command = Self {
            name_for_argv0: None,
            empty_environment: false,
            exec_as_login: false,
            args: Vec::new(),
        };

        let mut index = 0;
        while index < parsed_args.len() {
            let arg = &parsed_args[index];
            if arg == "--" {
                index += 1;
                break;
            }
            if arg == "-" || !arg.starts_with('-') {
                break;
            }

            let flags = &arg[1..];
            if flags.is_empty() {
                break;
            }

            for (offset, flag) in flags.char_indices() {
                match flag {
                    'a' => {
                        let value_start = offset + flag.len_utf8();
                        if value_start < flags.len() {
                            command.name_for_argv0 = Some(flags[value_start..].to_owned());
                        } else {
                            index += 1;
                            let value = parsed_args.get(index).ok_or_else(|| {
                                "exec: -a: option requires an argument".to_owned()
                            })?;
                            command.name_for_argv0 = Some(value.clone());
                        }
                        break;
                    }
                    'c' => command.empty_environment = true,
                    'l' => command.exec_as_login = true,
                    _ => return Err(format!("exec: -{flag}: invalid option")),
                }
            }

            index += 1;
        }

        command.args = parsed_args[index..].to_vec();
        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        if self.args.is_empty() {
            // When no arguments are present, then there's nothing for us to execute -- but we need
            // to ensure that any redirections setup for this builtin get applied to the calling
            // shell instance.
            #[allow(clippy::needless_collect)]
            let fds: Vec<_> = context.iter_fds().collect();

            context.shell.replace_open_files(fds.into_iter());
            return Ok(ExecutionResult::success());
        }

        // If we know we're already running in a subshell, then `exec`ing is actually
        // unsafe, since it would also replace the *parent* shell instance. We instead
        // delegate to the `command` builtin to perform the execution, with an expectation
        // of returning.
        if context.shell.is_subshell() {
            if self.empty_environment || self.exec_as_login || self.name_for_argv0.is_some() {
                return crate::engine::error::unimp(
                    "exec with options in subshell not yet supported",
                );
            }

            let cmd_cmd = crate::builtins::command::CommandCommand {
                command_and_args: self.args.clone(),
                ..Default::default()
            };

            return cmd_cmd.execute(context).await;
        }

        let mut argv0 = Cow::Borrowed(self.name_for_argv0.as_ref().unwrap_or(&self.args[0]));

        if self.exec_as_login {
            argv0 = Cow::Owned(std::format!("-{argv0}"));
        }

        let mut cmd = commands::compose_std_command(
            &context,
            &self.args[0],
            argv0.as_str(),
            &self.args[1..],
            self.empty_environment,
        )?;

        let status = cmd.status()?;
        let exit_code = status
            .code()
            .map_or(crate::engine::exit_code::CANNOT_EXECUTE, |code| {
                #[expect(clippy::cast_sign_loss)]
                let truncated = (code & 0xFF) as u8;
                truncated
            });

        let mut result = ExecutionResult::new(exit_code);
        result.next_control_flow = crate::engine::ExecutionControlFlow::ExitShell;
        Ok(result)
    }
}
