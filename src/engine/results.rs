//! Encapsulation of execution results.

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
#[derive(Default)]
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

        tracing::error!("unhandled process exit");
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

/// Represents the result of spawning an execution; captures both execution
/// that immediately returns as well as execution that starts a process
/// asynchronously.
pub enum ExecutionSpawnResult {
    /// Indicates that the execution completed.
    Completed(ExecutionResult),
    /// Indicates that a process was started and had not yet completed.
    StartedProcess(processes::ChildProcess),
    /// Indicates that a task was started to handle the execution asynchronously.
    StartedTask(compio::runtime::JoinHandle<Result<ExecutionResult, error::Error>>),
}

impl From<ExecutionResult> for ExecutionSpawnResult {
    fn from(result: ExecutionResult) -> Self {
        Self::Completed(result)
    }
}

impl ExecutionSpawnResult {
    /// Waits for the command to complete.
    pub async fn wait(self) -> Result<ExecutionWaitResult, error::Error> {
        let result = match self {
            Self::StartedProcess(mut child) => {
                // Wait for the process to exit or for a relevant signal, whichever happens
                // first.
                match child.wait().await? {
                    processes::ProcessWaitResult::Completed(output) => {
                        ExecutionWaitResult::Completed(ExecutionResult::from(output))
                    }
                    processes::ProcessWaitResult::Stopped => ExecutionWaitResult::Stopped(child),
                }
            }
            Self::Completed(result) => ExecutionWaitResult::Completed(result),
            Self::StartedTask(join_handle) => {
                let result = join_handle
                    .await
                    .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))?;
                ExecutionWaitResult::Completed(result?)
            }
        };

        Ok(result)
    }

    pub(crate) async fn poll(self) -> Result<ExecutionWaitResult, error::Error> {
        let result = match self {
            Self::StartedProcess(child) => ExecutionWaitResult::Stopped(child),
            Self::Completed(result) => ExecutionWaitResult::Completed(result),
            Self::StartedTask(join_handle) => {
                // TODO(jobs): This isn't right.
                let result = join_handle
                    .await
                    .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))?;
                ExecutionWaitResult::Completed(result?)
            }
        };

        Ok(result)
    }
}

/// Represents the result of waiting for an execution to complete.
pub enum ExecutionWaitResult {
    /// Indicates that the execution completed.
    Completed(ExecutionResult),
    /// Indicates that the execution was stopped.
    Stopped(processes::ChildProcess),
}
