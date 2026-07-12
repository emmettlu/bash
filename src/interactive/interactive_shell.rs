use std::io::IsTerminal as _;
use std::io::Write as _;

use crate::interactive::InputBackend;
use crate::interactive::InteractivePrompt;
use crate::interactive::ReadResult;
use crate::interactive::ShellError;

/// Result of an interactive execution.
pub enum InteractiveExecutionResult {
    /// The command was executed and returned the given result.
    Executed(crate::engine::ExecutionResult),
    /// The command failed to execute.
    Failed(crate::engine::Error),
    /// End of input was reached.
    Eof,
}

impl From<&InteractiveExecutionResult> for i32 {
    /// Converts an `InteractiveExecutionResult` into a signed, 32-bit exit code.
    fn from(value: &InteractiveExecutionResult) -> Self {
        match value {
            InteractiveExecutionResult::Executed(result) => result.exit_code.into(),
            InteractiveExecutionResult::Failed(_) => 1,
            InteractiveExecutionResult::Eof => 0,
        }
    }
}

/// Represents an interactive shell that displays prompts, interactively reads user input, etc.
pub struct InteractiveShell<'a, IB: InputBackend> {
    /// The underlying shell instance.
    shell: &'a mut crate::engine::Shell,
    /// The input backend to use.
    input: &'a mut IB,
    /// 终端控制 guard, 当这个循环持有 controlling terminal 时存在。
    _terminal_control: Option<crate::engine::terminal::TerminalControl>,
    /// Terminal integration utility, if any.
    terminal_integration: Option<crate::interactive::term_integration::TerminalIntegration>,
    /// Options.
    options: crate::interactive::UIOptions,
}

impl<'a, IB: InputBackend> InteractiveShell<'a, IB> {
    /// Creates a new `InteractiveShell` wrapping the given shell instance.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell instance to wrap.
    /// * `input` - The input backend to use.
    /// * `options` - The user interface options to use.
    pub fn new(
        shell: &'a mut crate::engine::Shell,
        input: &'a mut IB,
        options: &crate::interactive::UIOptions,
    ) -> Result<Self, ShellError> {
        let stdin_is_terminal = std::io::stdin().is_terminal();

        // Acquire terminal control if stdin is a terminal.
        let terminal_control =
            if options.interactive_session && options.terminal_control && stdin_is_terminal {
                Some(crate::engine::terminal::TerminalControl::acquire()?)
            } else {
                None
            };

        // Set up terminal integration if enabled *and* if stdin is a terminal.
        let terminal_integration = if options.interactive_session
            && options.terminal_shell_integration
            && stdin_is_terminal
        {
            let terminfo = crate::interactive::term_detection::get_terminal_info(&HostEnvironment);
            let terminal_integration =
                crate::interactive::term_integration::TerminalIntegration::new(terminfo);

            print!("{}", terminal_integration.initialize().as_ref());
            std::io::stdout().flush()?;

            Some(terminal_integration)
        } else {
            None
        };

        Ok(Self {
            shell,
            input,
            _terminal_control: terminal_control,
            terminal_integration,
            options: options.clone(),
        })
    }

    /// Runs the interactive shell loop, reading commands from standard input and writing
    /// results to standard output and standard error. Continues until the shell
    /// normally exits or until a fatal error occurs.
    pub async fn run_interactively(&mut self) -> Result<(), ShellError> {
        let mut announce_exit =
            self.options.interactive_session && self.shell.options().interactive;
        let mut session_started = false;

        let mut final_result = async {
            if self.options.interactive_session {
                self.shell.start_interactive_session()?;
                session_started = true;
            }

            loop {
                let result = self.run_interactively_once().await?;
                match result {
                    InteractiveExecutionResult::Executed(crate::engine::ExecutionResult {
                        next_control_flow: crate::engine::results::ExecutionControlFlow::ExitShell,
                        ..
                    })
                    | InteractiveExecutionResult::Eof => break,
                    InteractiveExecutionResult::Executed(crate::engine::ExecutionResult {
                        next_control_flow:
                            crate::engine::results::ExecutionControlFlow::ReturnFromFunctionOrScript,
                        ..
                    }) => {
                        log::error!("return from non-function/script");
                    }
                    InteractiveExecutionResult::Executed(_) => {}
                    InteractiveExecutionResult::Failed(err) => {
                        let mut stderr = self.shell.stderr();
                        let _ = self.shell.display_error(&mut stderr, &err);
                    }
                }

                if self.shell.options().exit_after_one_command {
                    announce_exit = false;
                    break;
                }
            }

            Ok(())
        }
        .await;

        let loop_succeeded = final_result.is_ok();
        if session_started {
            Self::retain_first_error(
                &mut final_result,
                self.shell.end_interactive_session().map_err(Into::into),
            );

            if loop_succeeded && announce_exit {
                let announce_result = writeln!(self.shell.stderr(), "exit").map_err(Into::into);
                Self::retain_first_error(&mut final_result, announce_result);
            }
        }

        if self.options.interactive_session
            && let Err(err) = self.shell.save_history()
        {
            log::warn!("couldn't save history: {err}");
            let mut stderr = self.shell.stderr();
            if let Err(display_err) = writeln!(stderr, "bash: failed to save history: {err}") {
                log::warn!("couldn't display history save error: {display_err}");
            }
        }

        let on_exit_result = self.shell.on_exit().await.map_err(Into::into);
        Self::retain_first_error(&mut final_result, on_exit_result);
        final_result
    }

    /// 运行非交互 stdin 输入循环, 直到 EOF 或 shell 退出。
    pub async fn run_stdin_input_loop(&mut self) -> Result<(), ShellError> {
        let mut final_result = async {
            loop {
                let result = self.run_stdin_input_loop_once().await?;
                match result {
                    InteractiveExecutionResult::Executed(crate::engine::ExecutionResult {
                        next_control_flow: crate::engine::results::ExecutionControlFlow::ExitShell,
                        ..
                    })
                    | InteractiveExecutionResult::Eof => break,
                    InteractiveExecutionResult::Executed(crate::engine::ExecutionResult {
                        next_control_flow:
                            crate::engine::results::ExecutionControlFlow::ReturnFromFunctionOrScript,
                        ..
                    }) => {
                        log::error!("return from non-function/script");
                    }
                    InteractiveExecutionResult::Executed(_) => {}
                    InteractiveExecutionResult::Failed(err) => {
                        let mut stderr = self.shell.stderr();
                        let _ = self.shell.display_error(&mut stderr, &err);
                    }
                }

                if self.shell.options().exit_after_one_command {
                    break;
                }
            }
            Ok(())
        }
        .await;

        let on_exit_result = self.shell.on_exit().await.map_err(Into::into);
        Self::retain_first_error(&mut final_result, on_exit_result);
        final_result
    }

    fn retain_first_error(result: &mut Result<(), ShellError>, next: Result<(), ShellError>) {
        if result.is_ok() {
            *result = next;
        } else if let Err(err) = next {
            log::debug!("additional interactive cleanup error: {err}");
        }
    }

    /// Runs the interactive shell loop once, reading a single command from standard input.
    async fn run_interactively_once(&mut self) -> Result<InteractiveExecutionResult, ShellError> {
        // Run any pre-prompt actions.
        Self::run_pre_prompt_actions(self.shell, &self.options).await?;

        // Compose the prompt.
        let prompt = if self.options.display_prompt {
            Self::compose_prompt(self.shell, self.terminal_integration.as_ref()).await?
        } else {
            Self::empty_prompt()
        };

        // Read input.
        match self.input.read_line(self.shell, prompt).await? {
            ReadResult::Input(read_result) => {
                // We got a line of input -- execute it.
                self.execute_line(read_result, true /* user input */).await
            }
            ReadResult::BoundCommand(read_result) => {
                // We got a line that was bound to keybindings; execute it.
                self.execute_line(read_result, false /* user input */).await
            }
            ReadResult::Eof => {
                // We're done!
                Ok(InteractiveExecutionResult::Eof)
            }
            ReadResult::Interrupted => {
                // We were interrupted; report that appropriately.
                let result = crate::engine::ExecutionResult::interrupted();
                self.shell.set_last_exit_status(result.exit_code);
                Ok(InteractiveExecutionResult::Executed(result))
            }
        }
    }

    async fn run_stdin_input_loop_once(
        &mut self,
    ) -> Result<InteractiveExecutionResult, ShellError> {
        let prompt = Self::empty_prompt();

        match self.input.read_line(self.shell, prompt).await? {
            ReadResult::Input(read_result) | ReadResult::BoundCommand(read_result) => {
                self.execute_line(read_result, false /* user input */).await
            }
            ReadResult::Eof => Ok(InteractiveExecutionResult::Eof),
            ReadResult::Interrupted => {
                let result = crate::engine::ExecutionResult::interrupted();
                self.shell.set_last_exit_status(result.exit_code);
                Ok(InteractiveExecutionResult::Executed(result))
            }
        }
    }

    fn empty_prompt() -> InteractivePrompt {
        InteractivePrompt {
            prompt: String::new(),
            alt_side_prompt: String::new(),
            continuation_prompt: String::new(),
        }
    }

    async fn compose_prompt(
        shell: &mut crate::engine::Shell,
        terminal_integration: Option<&crate::interactive::term_integration::TerminalIntegration>,
    ) -> Result<InteractivePrompt, ShellError> {
        // Now that we've done that, compose the prompt.
        let mut prompt = InteractivePrompt {
            prompt: shell.compose_prompt().await?,
            alt_side_prompt: String::new(),
            continuation_prompt: shell.compose_continuation_prompt().await?,
        };

        if let Some(terminal_integration) = terminal_integration {
            let pre_prompt = terminal_integration.pre_prompt();
            let working_dir = terminal_integration.report_cwd(shell.working_dir());
            let post_prompt = terminal_integration.post_prompt();

            prompt.prompt = [
                pre_prompt.as_ref(),
                working_dir.as_ref(),
                prompt.prompt.as_str(),
                post_prompt.as_ref(),
            ]
            .concat();
        }

        Ok(prompt)
    }

    /// Executes the given line of input.
    ///
    /// # Arguments
    ///
    /// * `read_result` - The line of input to execute.
    /// * `user_input` - Whether the line came from direct user input (as opposed to a key binding,
    ///   say).
    async fn execute_line(
        &mut self,
        read_result: String,
        user_input: bool,
    ) -> Result<InteractiveExecutionResult, ShellError> {
        if read_result.trim().is_empty() {
            let exit_code = self.shell.last_exit_status();
            return Ok(InteractiveExecutionResult::Executed(
                crate::engine::ExecutionResult::new(exit_code),
            ));
        }

        // See if the the user interface has a non-empty read buffer.
        let buffer_info = self.input.get_read_buffer();

        // If the user interface did, in fact, have a non-empty read buffer,
        // then reflect it to the shell in case any shell code wants to
        // process and/or transform the buffer.
        let nonempty_buffer = if let Some((buffer, cursor)) = buffer_info {
            if !buffer.is_empty() {
                self.shell.set_edit_buffer(buffer, cursor)?;
                true
            } else {
                false
            }
        } else {
            false
        };

        // If the line came from direct user input (as opposed to a key binding, say), then we
        // need to do a few more things before executing it.
        if user_input {
            Self::run_pre_exec_actions(
                self.shell,
                read_result.as_str(),
                &self.options,
                self.terminal_integration.as_ref(),
            )
            .await?;
        }

        // Count the command's lines.
        let line_count = read_result.lines().count().max(1);

        // Execute the command.
        let params = self.shell.default_exec_params();
        let source_info = crate::engine::SourceInfo::from("main");
        let result = match self
            .shell
            .run_string(read_result, &source_info, &params)
            .await
        {
            Ok(result) => Ok(InteractiveExecutionResult::Executed(result)),
            Err(e) => Ok(InteractiveExecutionResult::Failed(e)),
        };

        // Update cumulative line counter based on actual lines in the command.
        self.shell.increment_interactive_line_offset(line_count);

        // See if the shell has input buffer state that we need to reflect back to
        // the user interface. It may be state that originally came from the user
        // interface, or it may be state that was programmatically generated by
        // the command we just executed.
        let mut buffer_and_cursor = self.shell.pop_edit_buffer()?;

        if buffer_and_cursor.is_none() && nonempty_buffer {
            buffer_and_cursor = Some((String::new(), 0));
        }

        if let Some((updated_buffer, updated_cursor)) = buffer_and_cursor {
            self.input.set_read_buffer(updated_buffer, updated_cursor);
        }

        // Invoke terminal integration.
        if let Some(terminal_integration) = &self.terminal_integration {
            let exit_code = result.as_ref().map_or(1, i32::from);
            print!(
                "{}",
                terminal_integration.post_exec_command(exit_code).as_ref()
            );
            std::io::stdout().flush()?;
        }

        result
    }

    async fn run_pre_prompt_actions(
        shell: &mut crate::engine::Shell,
        options: &crate::interactive::UIOptions,
    ) -> Result<(), ShellError> {
        // Check for any completed jobs.
        shell.check_for_completed_jobs()?;

        // If there's a variable called PROMPT_COMMAND, then run it first.
        if options.run_prompt_command
            && let Some(prompt_cmd_var) = shell.env_var("PROMPT_COMMAND")
        {
            match prompt_cmd_var.value() {
                crate::engine::ShellValue::String(cmd_str) => {
                    Self::run_pre_prompt_command(shell, cmd_str.to_owned()).await?;
                }
                crate::engine::ShellValue::IndexedArray(values) => {
                    let owned_values: Vec<_> = values.values().cloned().collect();
                    for cmd_str in owned_values {
                        Self::run_pre_prompt_command(shell, cmd_str).await?;
                    }
                }
                // Other types are ignored.
                _ => (),
            }
        }

        // Next, run any zsh-style `precmd_functions`.
        // TODO(precmd_functions): verify if we need to save/restore exit results.
        if options.run_cmd_exec_funcs {
            // If there's a variable called precmd_functions, then call them.
            if let Some(crate::engine::ShellValue::IndexedArray(precmd_funcs)) = shell
                .env_var("precmd_functions")
                .map(|var| var.value())
                .cloned()
            {
                for func_name in precmd_funcs.values() {
                    let _ = shell
                        .invoke_function(
                            func_name,
                            std::iter::empty::<&str>(),
                            &shell.default_exec_params(),
                        )
                        .await;
                }
            }
        }

        Ok(())
    }

    async fn run_pre_exec_actions(
        shell: &mut crate::engine::Shell,
        command_line: &str,
        options: &crate::interactive::UIOptions,
        terminal_integration: Option<&crate::interactive::term_integration::TerminalIntegration>,
    ) -> Result<(), ShellError> {
        // Display the pre-command prompt (if there is one).
        let precmd_prompt = shell.compose_precmd_prompt().await?;
        if !precmd_prompt.is_empty() {
            print!("{precmd_prompt}");
        }

        // Update history (if applicable).
        shell.add_to_history(command_line.trim_end_matches('\n'))?;

        // Next, run any zsh-style `preexec_functions`.
        // TODO(preexec_functions): verify if we need to save/restore exit results.
        if options.run_cmd_exec_funcs {
            // If there's a variable called preexec_functions, then call them.
            if let Some(crate::engine::ShellValue::IndexedArray(preexec_funcs)) = shell
                .env_var("preexec_functions")
                .map(|var| var.value())
                .cloned()
            {
                for func_name in preexec_funcs.values() {
                    let _ = shell
                        .invoke_function(func_name, &[command_line], &shell.default_exec_params())
                        .await;
                }
            }
        }

        // Invoke terminal integration.
        if let Some(terminal_integration) = terminal_integration {
            print!(
                "{}",
                terminal_integration.pre_exec_command(command_line).as_ref()
            );
            std::io::stdout().flush()?;
        }

        Ok(())
    }

    async fn run_pre_prompt_command(
        shell: &mut crate::engine::Shell,
        prompt_cmd: String,
    ) -> Result<(), ShellError> {
        // Save (and later restore) the last exit status.
        let prev_last_result = shell.last_exit_status();
        let prev_last_pipeline_statuses = shell.last_pipeline_statuses().to_vec();

        // Run the command.
        let params = shell.default_exec_params();
        let source_info = crate::engine::SourceInfo::from("PROMPT_COMMAND");
        let result = shell.run_string(prompt_cmd, &source_info, &params).await;

        // 错误路径也必须恢复 prompt 前的退出状态.
        *shell.last_pipeline_statuses_mut() = prev_last_pipeline_statuses;
        shell.set_last_exit_status(prev_last_result);

        result?;
        Ok(())
    }
}

/// Represents the host environment; used for terminal detection in conjunction
/// with the `TerminalEnvironment` trait.
struct HostEnvironment;

impl crate::interactive::term_detection::TerminalEnvironment for HostEnvironment {
    /// Gets the value of the given environment variable from the host process's
    /// OS environment variables. Returns `None` if the variable is not set.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the environment variable to get.
    fn get_env_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::future::{Future, ready};

    use super::*;

    struct FailingInputBackend;

    impl InputBackend for FailingInputBackend {
        fn read_line<'a>(
            &'a mut self,
            _shell: &'a mut crate::engine::Shell,
            _prompt: InteractivePrompt,
        ) -> impl Future<Output = Result<ReadResult, ShellError>> + 'a {
            ready(Err(std::io::Error::other("测试输入错误").into()))
        }
    }

    struct EofInputBackend;

    impl InputBackend for EofInputBackend {
        fn read_line<'a>(
            &'a mut self,
            _shell: &'a mut crate::engine::Shell,
            _prompt: InteractivePrompt,
        ) -> impl Future<Output = Result<ReadResult, ShellError>> + 'a {
            ready(Ok(ReadResult::Eof))
        }
    }

    #[compio::test]
    async fn input_error_still_ends_session_and_runs_exit_trap() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        shell
            .run_string(
                "trap 'CLEANUP_RAN=yes' EXIT",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        let options = crate::interactive::UIOptions::builder()
            .terminal_control(false)
            .display_prompt(false)
            .run_prompt_command(false)
            .build();
        let mut input = FailingInputBackend;
        let result = InteractiveShell::new(&mut shell, &mut input, &options)?
            .run_interactively()
            .await;

        assert!(result.is_err());
        assert!(shell.end_interactive_session().is_err());
        assert!(matches!(
            shell.env_var("CLEANUP_RAN").map(|variable| variable.value()),
            Some(crate::engine::ShellValue::String(value)) if value == "yes"
        ));
        Ok(())
    }

    #[compio::test]
    async fn history_save_error_is_visible_without_changing_exit_status() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let history_path = scratch.path().join("history-directory");
        std::fs::create_dir(&history_path)?;
        let stderr_path = scratch.path().join("stderr");
        let stderr_file = std::fs::File::create(&stderr_path)?;
        let mut shell = crate::engine::Shell::builder()
            .interactive(true)
            .do_not_inherit_env(true)
            .build()
            .await?;
        shell.set_env_global(
            "HISTFILE",
            crate::engine::ShellVariable::new(history_path.to_string_lossy().to_string()),
        )?;
        shell.add_to_history("echo visible")?;
        shell.set_last_exit_status(23);
        shell.replace_open_files(
            [(
                crate::engine::openfiles::OpenFiles::STDERR_FD,
                stderr_file.into(),
            )]
            .into_iter(),
        );

        let options = crate::interactive::UIOptions::builder()
            .terminal_control(false)
            .display_prompt(false)
            .run_prompt_command(false)
            .build();
        let mut input = EofInputBackend;
        let result = InteractiveShell::new(&mut shell, &mut input, &options)?
            .run_interactively()
            .await;
        shell.replace_open_files(std::iter::empty());

        assert!(result.is_ok());
        assert_eq!(shell.last_exit_status(), 23);
        assert!(shell.history().unwrap().iter().any(|item| item.dirty));
        assert!(std::fs::read_to_string(stderr_path)?.contains("failed to save history"));
        Ok(())
    }
}
