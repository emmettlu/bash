//! Encapsulation of execution results.

use futures::FutureExt;
use std::sync::{Arc, Mutex};

use crate::engine::{error, processes};

/// 常见退出码常量.
pub mod exit_code {
    pub const SUCCESS: u8 = 0;
    pub const GENERAL_ERROR: u8 = 1;
    pub const INVALID_USAGE: u8 = 2;
    pub const UNIMPLEMENTED: u8 = 99;
    pub const CANNOT_EXECUTE: u8 = 126;
    pub const NOT_FOUND: u8 = 127;
    pub const INTERRUPTED: u8 = 130;
    pub const BROKEN_PIPE: u8 = 141;
}

/// Represents the result of executing a command or similar item.
#[derive(Clone, Default)]
pub struct ExecutionResult {
    /// The control flow transition to apply after execution.
    pub next_control_flow: ExecutionControlFlow,
    /// The exit code resulting from execution.
    pub exit_code: u8,
}

impl ExecutionResult {
    /// Returns a new `ExecutionResult` with the given exit code.
    ///
    /// # Arguments
    ///
    /// * `exit_code` - The exit code of the command.
    pub const fn new(exit_code: u8) -> Self {
        Self {
            exit_code,
            next_control_flow: ExecutionControlFlow::Normal,
        }
    }

    /// Returns a new `ExecutionResult` reflecting a process that was stopped.
    pub fn stopped() -> Self {
        // TODO(jobs): Replace this hardcoded compatibility signal value.
        const SIGTSTP: std::os::raw::c_int = 20;

        #[expect(clippy::cast_possible_truncation)]
        Self::new(128 + SIGTSTP as u8)
    }

    /// Returns a new `ExecutionResult` with an exit code of 0.
    pub const fn success() -> Self {
        Self::new(exit_code::SUCCESS)
    }

    /// Returns a new `ExecutionResult` with a general error exit code.
    pub const fn general_error() -> Self {
        Self::new(exit_code::GENERAL_ERROR)
    }

    /// Returns a new `ExecutionResult` with an invalid usage exit code.
    pub const fn invalid_usage() -> Self {
        Self::new(exit_code::INVALID_USAGE)
    }

    /// Returns a new `ExecutionResult` with a not-found exit code.
    pub const fn not_found() -> Self {
        Self::new(exit_code::NOT_FOUND)
    }

    /// Returns a new `ExecutionResult` with a cannot-execute exit code.
    pub const fn cannot_execute() -> Self {
        Self::new(exit_code::CANNOT_EXECUTE)
    }

    /// Returns a new `ExecutionResult` with an unimplemented exit code.
    pub const fn unimplemented() -> Self {
        Self::new(exit_code::UNIMPLEMENTED)
    }

    /// Returns a new `ExecutionResult` with an interrupted exit code.
    pub const fn interrupted() -> Self {
        Self::new(exit_code::INTERRUPTED)
    }

    /// Returns whether the command was successful.
    pub const fn is_success(&self) -> bool {
        self.exit_code == exit_code::SUCCESS
    }

    /// Returns whether the execution result indicates normal control flow.
    /// Returns `false` if there is any control flow transition requested.
    pub const fn is_normal_flow(&self) -> bool {
        matches!(self.next_control_flow, ExecutionControlFlow::Normal)
    }

    /// Returns whether the execution result indicates a loop break.
    pub const fn is_break(&self) -> bool {
        matches!(
            self.next_control_flow,
            ExecutionControlFlow::BreakLoop { .. }
        )
    }

    /// Returns whether the execution result indicates a loop continue.
    pub const fn is_continue(&self) -> bool {
        matches!(
            self.next_control_flow,
            ExecutionControlFlow::ContinueLoop { .. }
        )
    }

    /// Returns whether the execution result indicates an early return
    /// from a function or script, or an exit from the shell. Returns `false`
    /// otherwise, including loop breaks or continues.
    pub const fn is_return_or_exit(&self) -> bool {
        matches!(
            self.next_control_flow,
            ExecutionControlFlow::ReturnFromFunctionOrScript | ExecutionControlFlow::ExitShell
        )
    }
}

impl From<ExecutionWaitResult> for ExecutionResult {
    fn from(wait_result: ExecutionWaitResult) -> Self {
        match wait_result {
            ExecutionWaitResult::Completed(result) => result,
            ExecutionWaitResult::Running(..) => Self::success(),
            // TODO(jobs): We need to job-manage the stopped process.
            ExecutionWaitResult::Stopped(..) => Self::stopped(),
        }
    }
}

impl From<std::process::Output> for ExecutionResult {
    fn from(output: std::process::Output) -> Self {
        if let Some(code) = output.status.code() {
            #[expect(clippy::cast_sign_loss)]
            return Self::new((code & 0xFF) as u8);
        }

        log::error!("unhandled process exit");
        Self::new(exit_code::NOT_FOUND)
    }
}

/// Represents a control flow transition to apply.
#[derive(Clone, Copy, Default)]
pub enum ExecutionControlFlow {
    /// Continue normal execution.
    #[default]
    Normal,
    /// Break out of an enclosing loop.
    BreakLoop {
        /// Identifies which level of nested loops to break out of. 0 indicates the innermost loop,
        /// 1 indicates the next outer loop, and so on.
        levels: usize,
    },
    /// Continue to the next iteration of an enclosing loop.
    ContinueLoop {
        /// Identifies which level of nested loops to continue. 0 indicates the innermost loop,
        /// 1 indicates the next outer loop, and so on.
        levels: usize,
    },
    /// Return from the current function or script.
    ReturnFromFunctionOrScript,
    /// Exit the shell.
    ExitShell,
}

impl ExecutionControlFlow {
    /// Attempts to decrement the loop levels for `BreakLoop` or `ContinueLoop`.
    /// If the levels reach zero, transitions to `Normal`. If the control flow is not
    /// a loop break or continue, no changes are made.
    #[must_use]
    pub const fn try_decrement_loop_levels(&self) -> Self {
        match self {
            Self::BreakLoop { levels: 0 } | Self::ContinueLoop { levels: 0 } => Self::Normal,
            Self::BreakLoop { levels } => Self::BreakLoop {
                levels: *levels - 1,
            },
            Self::ContinueLoop { levels } => Self::ContinueLoop {
                levels: *levels - 1,
            },
            control_flow => *control_flow,
        }
    }
}

/// 与消费端执行上下文共享生命周期的 process substitution task.
#[derive(Clone)]
pub(crate) struct ProcessSubstitutionTask {
    state: Arc<Mutex<ProcessSubstitutionTaskState>>,
}

struct ProcessSubstitutionTaskState {
    task: Option<ExecutionTask>,
}

impl ProcessSubstitutionTask {
    /// 创建一个由消费端生命周期管理的 producer task.
    pub(crate) fn new(task: ExecutionTask) -> Self {
        Self {
            state: Arc::new(Mutex::new(ProcessSubstitutionTaskState {
                task: Some(task),
            })),
        }
    }

    fn take_task(&self) -> Option<ExecutionTask> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .task
            .take()
    }

    /// 等待 producer 完成, 并保留执行错误.
    pub(crate) async fn wait(&self) -> Result<(), error::Error> {
        let Some(mut task) = self.take_task() else {
            return Ok(());
        };

        // producer 可递归创建 process substitution, 这里用装箱切断 future 类型递归.
        let result = Box::pin(task.wait()).await?;
        if !result.is_success() {
            log::debug!(
                "process substitution producer exited with status {}",
                result.exit_code
            );
        }
        Ok(())
    }

    /// 非阻塞轮询 producer 是否完成.
    fn poll(&self) -> Option<Result<(), error::Error>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(task) = state.task.as_mut() else {
            return Some(Ok(()));
        };
        let result = task.poll()?;
        state.task.take();
        Some(result.map(|result| {
            if !result.is_success() {
                log::debug!(
                    "process substitution producer exited with status {}",
                    result.exit_code
                );
            }
        }))
    }

    /// 取消尚未完成的 producer.
    pub(crate) fn cancel(&self) -> Result<(), error::Error> {
        self.take_task().map_or(Ok(()), ExecutionTask::cancel)
    }
}

impl Drop for ProcessSubstitutionTaskState {
    fn drop(&mut self) {
        let Some(mut task) = self.task.take() else {
            return;
        };

        if let Some(result) = task.poll() {
            match result {
                Ok(result) if !result.is_success() => log::debug!(
                    "process substitution producer exited with status {}",
                    result.exit_code
                ),
                Err(err) => log::debug!("process substitution producer failed: {err}"),
                Ok(_) => {}
            }
        }
    }
}

/// 可取消的 owned shell task.
pub struct ExecutionTask {
    handle: compio::runtime::JoinHandle<Result<ExecutionResult, error::Error>>,
    cancellation: processes::TaskCancellation,
    process_substitutions: Vec<ProcessSubstitutionTask>,
    process_substitution_error: Option<error::Error>,
    completion_result: Option<Result<ExecutionResult, error::Error>>,
    armed: bool,
}

impl ExecutionTask {
    /// 包装 task, 并在异常 drop 时触发外部进程取消.
    pub(crate) const fn new(
        handle: compio::runtime::JoinHandle<Result<ExecutionResult, error::Error>>,
        cancellation: processes::TaskCancellation,
    ) -> Self {
        Self {
            handle,
            cancellation,
            process_substitutions: Vec::new(),
            process_substitution_error: None,
            completion_result: None,
            armed: true,
        }
    }

    /// 将 process substitution producer 绑定到消费 task.
    pub(crate) fn add_process_substitutions(
        &mut self,
        tasks: impl IntoIterator<Item = ProcessSubstitutionTask>,
    ) {
        self.process_substitutions.extend(tasks);
    }

    fn cancel_process_substitutions(&mut self) -> Result<(), error::Error> {
        let mut first_error = None;
        for task in self.process_substitutions.drain(..) {
            if let Err(err) = task.cancel()
                && first_error.is_none()
            {
                first_error = Some(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn wait_for_process_substitutions(&mut self) -> Result<(), error::Error> {
        let mut first_error = self.process_substitution_error.take();
        for task in self.process_substitutions.drain(..) {
            if let Err(err) = task.wait().await
                && first_error.is_none()
            {
                first_error = Some(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// 主动取消 task 及其已注册的外部进程.
    pub(crate) fn cancel(mut self) -> Result<(), error::Error> {
        let task_result = self.cancellation.cancel();
        let substitution_result = self.cancel_process_substitutions();
        self.armed = false;
        task_result.and(substitution_result)
    }

    /// 等待 task 完成.
    pub(crate) async fn wait(&mut self) -> Result<ExecutionResult, error::Error> {
        let task_result = if let Some(result) = self.completion_result.take() {
            result
        } else {
            let result = (&mut self.handle)
                .await
                .map_err(|err| {
                    error::Error::from(error::ErrorKind::ThreadingError(err.to_string()))
                })
                .and_then(|result| result);
            self.armed = false;
            result
        };

        match task_result {
            Ok(result) if result.is_success() => {
                self.wait_for_process_substitutions().await?;
                Ok(result)
            }
            Ok(result) => {
                if let Err(cleanup_err) = self.cancel_process_substitutions() {
                    log::debug!(
                        "failed to clean up process substitution after consumer failure: {cleanup_err}"
                    );
                }
                Ok(result)
            }
            Err(err) => {
                if let Err(cleanup_err) = self.cancel_process_substitutions() {
                    log::debug!(
                        "failed to clean up process substitution after consumer error: {cleanup_err}"
                    );
                }
                Err(err)
            }
        }
    }

    /// 非阻塞轮询 task 是否完成.
    pub(crate) fn poll(&mut self) -> Option<Result<ExecutionResult, error::Error>> {
        if self.completion_result.is_none() {
            let result = (&mut self.handle).now_or_never()?;
            self.armed = false;
            self.completion_result = Some(
                result
                    .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()).into())
                    .and_then(|result| result),
            );
        }

        if self
            .completion_result
            .as_ref()
            .is_some_and(|result| match result {
                Ok(result) => !result.is_success(),
                Err(_) => true,
            })
        {
            if let Err(err) = self.cancel_process_substitutions() {
                log::debug!(
                    "failed to clean up process substitution after consumer failure: {err}"
                );
            }
            return self.completion_result.take();
        }

        let mut first_error = self.process_substitution_error.take();
        for task in &self.process_substitutions {
            match task.poll() {
                Some(Ok(())) => {}
                Some(Err(err)) => {
                    if first_error.is_none() {
                        first_error = Some(err);
                    }
                }
                None => {
                    self.process_substitution_error = first_error;
                    return None;
                }
            }
        }
        self.process_substitutions.clear();

        if let Some(err) = first_error {
            Some(Err(err))
        } else {
            self.completion_result.take()
        }
    }
}

impl Drop for ExecutionTask {
    fn drop(&mut self) {
        if self.armed
            && let Err(err) = self.cancellation.cancel()
        {
            log::debug!("failed to cancel owned shell task: {err}");
        }
        if let Err(err) = self.cancel_process_substitutions() {
            log::debug!("failed to cancel process substitution producer: {err}");
        }
    }
}

/// Represents the result of spawning an execution; captures both execution
/// that immediately returns as well as execution that starts a process
/// asynchronously.
pub enum ExecutionSpawnResult {
    /// Indicates that the execution completed.
    Completed(ExecutionResult),
    /// Indicates that a process was started and had not yet completed.
    StartedProcess(processes::ChildProcess),
    /// Indicates that a task was started to handle the execution asynchronously.
    StartedTask(ExecutionTask),
}

impl From<ExecutionResult> for ExecutionSpawnResult {
    fn from(result: ExecutionResult) -> Self {
        Self::Completed(result)
    }
}

impl ExecutionSpawnResult {
    /// 将 process substitution producer 绑定到消费命令.
    pub(crate) async fn with_process_substitutions(
        self,
        tasks: Vec<ProcessSubstitutionTask>,
    ) -> Result<Self, error::Error> {
        if tasks.is_empty() {
            return Ok(self);
        }

        match self {
            Self::Completed(result) if result.is_success() => {
                for task in tasks {
                    task.wait().await?;
                }
                Ok(Self::Completed(result))
            }
            Self::Completed(result) => {
                for task in tasks {
                    if let Err(err) = task.cancel() {
                        log::debug!(
                            "failed to clean up process substitution after consumer failure: {err}"
                        );
                    }
                }
                Ok(Self::Completed(result))
            }
            Self::StartedTask(mut task) => {
                task.add_process_substitutions(tasks);
                Ok(Self::StartedTask(task))
            }
            Self::StartedProcess(process) => {
                for task in tasks {
                    if let Err(err) = task.cancel() {
                        log::debug!(
                            "failed to clean up unsupported external process substitution: {err}"
                        );
                    }
                }
                if let Err(err) = process.terminate_and_reap() {
                    log::debug!("failed to clean up external consumer process: {err}");
                }
                Err(error::ErrorKind::NotSupported(
                    "process substitution with external commands on Windows",
                )
                .into())
            }
        }
    }

    /// 取消或终止仍在运行的执行单元, 并在需要时后台回收进程.
    pub(crate) fn cleanup(self) -> Result<(), error::Error> {
        match self {
            Self::Completed(_) => Ok(()),
            Self::StartedProcess(child) => child.terminate_and_reap(),
            // 先终止 task 注册的外部进程, 再由 drop 取消异步执行.
            Self::StartedTask(task) => task.cancel(),
        }
    }

    /// Waits for the command to complete.
    pub async fn wait(self) -> Result<ExecutionWaitResult, error::Error> {
        let result = match self {
            Self::StartedProcess(mut child) => child.wait().await?.into_wait_result(child),
            Self::Completed(result) => ExecutionWaitResult::Completed(result),
            Self::StartedTask(mut task) => ExecutionWaitResult::Completed(task.wait().await?),
        };

        Ok(result)
    }

    pub(crate) async fn poll(self) -> Result<ExecutionWaitResult, error::Error> {
        let result = match self {
            Self::StartedProcess(mut child) => match child.poll() {
                Some(result) => ExecutionWaitResult::Completed(ExecutionResult::from(result?)),
                None => ExecutionWaitResult::Running(child),
            },
            Self::Completed(result) => ExecutionWaitResult::Completed(result),
            Self::StartedTask(mut task) => ExecutionWaitResult::Completed(task.wait().await?),
        };

        Ok(result)
    }
}

/// Represents the result of waiting for an execution to complete.
pub enum ExecutionWaitResult {
    /// Indicates that the execution completed.
    Completed(ExecutionResult),
    /// Indicates that the execution is still running.
    Running(processes::ChildProcess),
    /// Indicates that the execution was stopped.
    Stopped(processes::ChildProcess),
}

impl processes::ProcessWaitResult {
    fn into_wait_result(self, child: processes::ChildProcess) -> ExecutionWaitResult {
        match self {
            Self::Completed(output) => {
                ExecutionWaitResult::Completed(ExecutionResult::from(output))
            }
            Self::Stopped => ExecutionWaitResult::Stopped(child),
        }
    }
}
