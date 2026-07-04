//! Process management

use crate::engine::{error, sys};

/// Tracks a child process being awaited.
pub struct ChildProcess {
    /// 尚未回收的子进程.
    child: Option<sys::process::Child>,
    /// If available, the process ID of the child.
    pid: Option<sys::process::ProcessId>,
    /// If available, the process group ID of the child.
    pgid: Option<sys::process::ProcessId>,
}

impl ChildProcess {
    /// 包装子进程, 但不立即启动后台等待线程.
    pub fn new(
        child: sys::process::Child,
        pid: Option<sys::process::ProcessId>,
        pgid: Option<sys::process::ProcessId>,
    ) -> Self {
        Self {
            child: Some(child),
            pid,
            pgid,
        }
    }

    /// Returns the process's ID.
    pub const fn pid(&self) -> Option<sys::process::ProcessId> {
        self.pid
    }

    /// Returns the process's group ID.
    pub const fn pgid(&self) -> Option<sys::process::ProcessId> {
        self.pgid
    }

    /// Waits for the process to exit without blocking the async runtime thread.
    pub async fn wait(&mut self) -> Result<ProcessWaitResult, error::Error> {
        let Some(mut child) = self.child.take() else {
            return Err(error::ErrorKind::InternalError("process already waited".into()).into());
        };

        let status = compio::runtime::spawn_blocking(move || child.wait())
            .await
            .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))??;

        Ok(ProcessWaitResult::Completed(output_from_status(status)))
    }

    /// Polls the process for completion without blocking.
    pub(crate) fn poll(&mut self) -> Option<Result<std::process::Output, error::Error>> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                self.child = None;
                Some(Ok(output_from_status(status)))
            }
            Ok(None) => None,
            Err(err) => {
                self.child = None;
                Some(Err(err.into()))
            }
        }
    }
}

fn output_from_status(status: sys::process::ExitStatus) -> std::process::Output {
    std::process::Output {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

/// Represents the result of waiting for an executing process.
pub enum ProcessWaitResult {
    /// The process completed.
    Completed(std::process::Output),
    /// The process stopped and has not yet completed.
    Stopped,
}
