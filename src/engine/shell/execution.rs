//! Execution support for shell.

use std::path::{Path, PathBuf};

use crate::engine::{
    ExecutionControlFlow, ExecutionParameters, ExecutionResult, ProcessGroupPolicy, SourceInfo,
    arithmetic::Evaluatable as _, callstack, error, interp::Execute as _, jobs, openfiles, sys,
    trace_categories,
};

fn script_fd(path: &Path) -> Option<crate::engine::ShellFd> {
    if path.parent() != Some(Path::new("/dev/fd")) {
        return None;
    }

    path.file_name()?.to_string_lossy().parse().ok()
}

fn open_and_parse_script(
    requested_path: PathBuf,
    absolute_path: PathBuf,
    inherited_file: Option<openfiles::OpenFile>,
    parser_options: crate::parser::ParserOptions,
    source_name: String,
) -> Result<Result<crate::parser::ast::Program, crate::parser::ParseError>, std::io::Error> {
    let opened_file = if let Some(file) = inherited_file {
        file
    } else if let Some(result) = sys::fs::try_open_special_file(&requested_path) {
        result?.into()
    } else {
        std::fs::File::open(absolute_path)?.into()
    };

    if opened_file.is_dir() {
        return Err(std::io::ErrorKind::IsADirectory.into());
    }

    log::debug!(target: trace_categories::PARSE, "Parsing sourced file: {source_name}");
    let reader = std::io::BufReader::new(opened_file);
    let mut parser = crate::parser::Parser::new(reader, &parser_options);
    Ok(parser.parse_program())
}

impl crate::engine::Shell {
    /// 尝试创建用于子 shell 语义的 shell 副本.
    ///
    /// fd 复制失败会作为错误返回. 子 shell 不继承交互 history、key bindings
    /// 或 executable completion cache, 避免复制与执行语义无关的状态.
    pub(crate) fn try_fork_subshell(&self) -> Result<Self, error::Error> {
        let mut call_stack = self.call_stack.clone();
        call_stack.clear_active_trap_signals();

        Ok(Self {
            error_formatter: self.error_formatter.clone(),
            traps: self.traps.clone(),
            open_files: self.open_files.try_clone()?,
            working_dir: self.working_dir.clone(),
            env: self.env.clone(),
            funcs: self.funcs.clone(),
            options: self.options.clone(),
            jobs: jobs::JobManager::new(),
            aliases: self.aliases.clone(),
            last_exit_status: self.last_exit_status,
            last_exit_status_change_count: self.last_exit_status_change_count,
            last_pipeline_statuses: self.last_pipeline_statuses.clone(),
            depth: self.depth + 1,
            name: self.name.clone(),
            args: self.args.clone(),
            version: self.version.clone(),
            product_display_str: self.product_display_str.clone(),
            call_stack,
            directory_stack: self.directory_stack.clone(),
            completion_config: self.completion_config.clone(),
            builtins: self.builtins.clone(),
            program_location_cache: self.program_location_cache.clone(),
            external_command_completion_cache: Default::default(),
            key_bindings: None,
            history: None,
        })
    }

    /// Returns the default execution parameters for this shell.
    pub fn default_exec_params(&self) -> ExecutionParameters {
        let mut params = ExecutionParameters::default();

        params.process_group_policy = if self.options.enable_job_control {
            ProcessGroupPolicy::NewProcessGroup
        } else {
            ProcessGroupPolicy::SameProcessGroup
        };

        params
    }

    pub(super) async fn source_if_exists(
        &mut self,
        path: impl AsRef<Path>,
        params: &ExecutionParameters,
    ) -> Result<bool, error::Error> {
        let path = path.as_ref().to_owned();
        let worker_path = path.clone();
        let exists = compio::runtime::spawn_blocking(move || worker_path.exists())
            .await
            .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))?;
        if exists {
            self.source_script(&path, std::iter::empty::<String>(), params)
                .await?;
            Ok(true)
        } else {
            log::debug!("skipping non-existent file: {}", path.display());
            Ok(false)
        }
    }

    /// Source the given file as a shell script, returning the execution result.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the file to source.
    /// * `args` - The arguments to pass to the script as positional parameters.
    /// * `params` - Execution parameters.
    pub async fn source_script<S: Into<String>, P: AsRef<Path>, I: Iterator<Item = S>>(
        &mut self,
        path: P,
        args: I,
        params: &ExecutionParameters,
    ) -> Result<ExecutionResult, error::Error> {
        self.execute_script_file(path, args, params, callstack::ScriptCallType::Source)
            .await
    }

    /// 打开并执行脚本文件, 将解析、调用栈管理和控制流边界处理合并在一处。
    async fn execute_script_file<S: Into<String>, P: AsRef<Path>, I: Iterator<Item = S>>(
        &mut self,
        path: P,
        args: I,
        params: &ExecutionParameters,
        call_type: callstack::ScriptCallType,
    ) -> Result<ExecutionResult, error::Error> {
        let path = path.as_ref().to_owned();
        log::debug!("sourcing: {}", path.display());

        let absolute_path = self.absolute_path(&path);
        let inherited_file = script_fd(&absolute_path)
            .and_then(|fd| params.fd_overlay(self).try_fd(fd))
            .map(openfiles::OpenFile::try_clone)
            .transpose()
            .map_err(|err| error::ErrorKind::FailedSourcingFile(path.clone(), err))?;
        let parser_options = self.parser_options();
        let source_info = crate::engine::SourceInfo::from(path.clone());
        let source_name = source_info.source.clone();
        let worker_path = path.clone();

        let parse_result = compio::runtime::spawn_blocking(move || {
            open_and_parse_script(
                worker_path,
                absolute_path,
                inherited_file,
                parser_options,
                source_name,
            )
        })
        .await
        .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))?
        .map_err(|err| error::ErrorKind::FailedSourcingFile(path, err))?;

        let script_positional_args = args.map(Into::into);
        self.call_stack
            .push_script(call_type, &source_info, script_positional_args);

        let mut result = self
            .run_parsed_result(parse_result, &source_info, params)
            .await;

        self.call_stack.pop();

        // 处理脚本执行边界处的 return 控制流: return 在此消费, 其余控制流原样保留
        if let Ok(ref mut r) = result
            && matches!(
                r.next_control_flow,
                ExecutionControlFlow::ReturnFromFunctionOrScript
            )
        {
            r.next_control_flow = ExecutionControlFlow::Normal;
        }

        result
    }

    /// Executes the given string as a shell program, returning the resulting exit status.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to execute.
    /// * `source_info` - Information about the source of the command text.
    /// * `params` - Execution parameters.
    pub async fn run_string<S: Into<String>>(
        &mut self,
        command: S,
        source_info: &crate::engine::SourceInfo,
        params: &ExecutionParameters,
    ) -> Result<ExecutionResult, error::Error> {
        let parse_result = self.parse_string(command);
        self.run_parsed_result(parse_result, source_info, params)
            .await
    }

    /// Executes the given command, provided to a shell executable on the command
    /// line (i.e., via `-c`).
    ///
    /// It is expected that the shell will not be used for any further execution
    /// after this command; this function will perform any necessary shell exit
    /// handling.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to execute.
    pub async fn run_dash_c_command<S: Into<String>>(
        &mut self,
        command: S,
    ) -> Result<ExecutionResult, error::Error> {
        self.start_command_string_mode();

        // Execute the command string.
        let params = self.default_exec_params();
        let source_info = SourceInfo::from("-c");
        let result = self.run_string(command, &source_info, &params).await?;

        self.end_command_string_mode()?;

        // Give the shell a chance to run on-exit tasks, but ignore the result.
        let _ = self.on_exit().await;

        Ok(result)
    }

    /// Executes the given script file, returning the resulting exit status.
    ///
    /// It is expected that the shell will not be used for any further execution
    /// after this command; this function will perform any necessary shell exit
    /// handling.
    ///
    /// # Arguments
    ///
    /// * `script_path` - The path to the script file to execute.
    /// * `args` - The arguments to pass to the script as positional parameters.
    pub async fn run_script<S: Into<String>, P: AsRef<Path>, I: Iterator<Item = S>>(
        &mut self,
        script_path: P,
        args: I,
    ) -> Result<ExecutionResult, error::Error> {
        let params = self.default_exec_params();
        let result = self
            .execute_script_file(script_path, args, &params, callstack::ScriptCallType::Run)
            .await?;

        // Give the shell a chance to run on-exit tasks, but ignore the result.
        let _ = self.on_exit().await;

        Ok(result)
    }

    pub(crate) async fn run_parsed_result(
        &mut self,
        parse_result: Result<crate::parser::ast::Program, crate::parser::ParseError>,
        source_info: &crate::engine::SourceInfo,
        params: &ExecutionParameters,
    ) -> Result<ExecutionResult, error::Error> {
        // If parsing succeeded, run the program. If there's a parse error, it's fatal (per spec).
        let result = match parse_result {
            Ok(prog) => self.run_program(prog, params).await,
            Err(parse_err) => Err(error::Error::from(error::ErrorKind::ParseError(
                parse_err,
                source_info.clone(),
            ))
            .into_fatal()),
        };

        // Report any errors.
        match result {
            Ok(result) => Ok(result),
            Err(err) => {
                let _ = self.display_error(&mut params.stderr(self), &err);

                let result = err.into_result(self);
                self.set_last_exit_status(result.exit_code);

                Ok(result)
            }
        }
    }

    /// Executes the given parsed shell program, returning the resulting exit status.
    ///
    /// # Arguments
    ///
    /// * `program` - The program to execute.
    /// * `params` - Execution parameters.
    pub async fn run_program(
        &mut self,
        program: crate::parser::ast::Program,
        params: &ExecutionParameters,
    ) -> Result<ExecutionResult, error::Error> {
        program.execute(self, params).await
    }

    /// Evaluate the given arithmetic expression, returning the result.
    pub fn eval_arithmetic(
        &mut self,
        expr: &crate::parser::ast::ArithmeticExpr,
    ) -> Result<i64, error::Error> {
        Ok(expr.eval(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::open_and_parse_script;
    use crate::engine::Shell;

    #[compio::test]
    async fn script_worker_returns_owned_parse_result() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("script.sh");
        std::fs::write(&path, "value=worker\n")?;
        let requested_path = path.clone();
        let absolute_path = path;

        let parse_result = compio::runtime::spawn_blocking(move || {
            open_and_parse_script(
                requested_path,
                absolute_path,
                None,
                crate::parser::ParserOptions::default(),
                String::from("script.sh"),
            )
        })
        .await
        .map_err(|err| anyhow::anyhow!("script worker panicked: {err:?}"))??;

        assert!(parse_result.is_ok());
        Ok(())
    }

    #[test]
    fn subshell_fork_omits_interactive_state() {
        let mut shell = Shell::empty();
        shell.history = Some(Default::default());
        let parent_names = shell.external_command_completion_cache.get_or_update(
            "parent-path".into(),
            ".EXE".into(),
            true,
            |_, _, _| vec!["parent.exe".into()],
        );
        assert_eq!(parent_names, ["parent.exe"]);

        let mut subshell = shell.try_fork_subshell().unwrap();

        assert_eq!(subshell.depth, shell.depth + 1);
        assert!(subshell.history.is_none());
        assert!(subshell.key_bindings.is_none());
        let subshell_names = subshell.external_command_completion_cache.get_or_update(
            "parent-path".into(),
            ".EXE".into(),
            true,
            |_, _, _| vec!["subshell.exe".into()],
        );
        assert_eq!(subshell_names, ["subshell.exe"]);

        let parent_names = shell.external_command_completion_cache.get_or_update(
            "parent-path".into(),
            ".EXE".into(),
            true,
            |_, _, _| vec!["unexpected.exe".into()],
        );
        assert_eq!(parent_names, ["parent.exe"]);
    }
}
