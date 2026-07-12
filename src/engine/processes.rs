//! Process management

use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::sync::{Arc, Mutex};

use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

use crate::engine::{error, sys};

/// 可跨 task 保留的进程终止句柄.
#[derive(Clone)]
struct ProcessControl {
    handle: Arc<OwnedHandle>,
}

impl ProcessControl {
    fn open(pid: sys::process::ProcessId) -> Result<Self, error::Error> {
        let pid = u32::try_from(pid).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid child process id")
        })?;
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(Self {
            handle: Arc::new(unsafe { OwnedHandle::from_raw_handle(handle) }),
        })
    }

    fn terminate(&self) -> Result<(), error::Error> {
        let handle = self.handle.as_raw_handle();
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT => {}
            WAIT_FAILED => return Err(std::io::Error::last_os_error().into()),
            result => {
                return Err(error::ErrorKind::InternalError(std::format!(
                    "unexpected process wait result: {result}"
                ))
                .into());
            }
        }

        if unsafe { TerminateProcess(handle, 1) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(())
    }
}

#[derive(Default)]
struct TaskCancellationState {
    cancelled: bool,
    processes: Vec<ProcessControl>,
}

/// 跟踪 owned shell task 启动的外部进程, 供 task 外层安全取消.
#[derive(Clone, Default)]
pub(crate) struct TaskCancellation {
    state: Arc<Mutex<TaskCancellationState>>,
}

impl TaskCancellation {
    /// 创建尚未取消的 task 控制器.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 注册 task 当前或后续等待的外部进程.
    pub(crate) fn register(&self, process: &ChildProcess) -> Result<(), error::Error> {
        let control = process.control.clone();
        let terminate_now = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.cancelled {
                true
            } else {
                state.processes.push(control.clone());
                false
            }
        };

        if terminate_now {
            control.terminate()?;
        }

        Ok(())
    }

    /// 标记 task 已取消并终止其注册的全部外部进程.
    pub(crate) fn cancel(&self) -> Result<(), error::Error> {
        let processes = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.cancelled = true;
            state.processes.clone()
        };

        let mut first_error = None;
        for process in processes {
            if let Err(err) = process.terminate()
                && first_error.is_none()
            {
                first_error = Some(err);
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    #[cfg(test)]
    pub(crate) fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancelled
    }
}

/// Tracks a child process being awaited.
pub struct ChildProcess {
    /// 尚未回收的子进程.
    child: Option<sys::process::Child>,
    /// 即使 child 已进入 blocking wait 仍可使用的终止句柄.
    control: ProcessControl,
    /// If available, the process ID of the child.
    pid: Option<sys::process::ProcessId>,
    /// If available, the process group ID of the child.
    pgid: Option<sys::process::ProcessId>,
}

impl ChildProcess {
    /// 包装子进程, 但不立即启动后台等待线程.
    pub fn new(
        child: sys::process::Child,
        pid: sys::process::ProcessId,
        pgid: Option<sys::process::ProcessId>,
    ) -> Result<Self, error::Error> {
        Ok(Self {
            child: Some(child),
            control: ProcessControl::open(pid)?,
            pid: Some(pid),
            pgid,
        })
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

        let wait_result = compio::runtime::spawn_blocking(move || child.wait()).await;
        let status = match wait_result {
            Ok(Ok(status)) => status,
            Ok(Err(err)) => {
                if let Err(terminate_err) = self.control.terminate() {
                    log::debug!("failed to terminate process after wait error: {terminate_err}");
                }
                return Err(err.into());
            }
            Err(err) => {
                if let Err(terminate_err) = self.control.terminate() {
                    log::debug!("failed to terminate process after join error: {terminate_err}");
                }
                return Err(error::ErrorKind::ThreadingError(err.to_string()).into());
            }
        };

        Ok(ProcessWaitResult::Completed(output_from_status(status)))
    }

    /// 终止进程, 但保留 child handle 供后续回收.
    pub(crate) fn terminate(&mut self) -> Result<(), error::Error> {
        if let Some(result) = self.poll() {
            return result.map(|_| ());
        }

        self.control.terminate()
    }

    /// 终止进程并在后台回收, 不阻塞当前错误返回路径.
    pub(crate) fn terminate_and_reap(mut self) -> Result<(), error::Error> {
        let terminate_result = self.terminate();

        if self.child.is_some() {
            compio::runtime::spawn(async move {
                if let Err(err) = self.wait().await {
                    log::debug!("failed to reap terminated pipeline process: {err}");
                }
            })
            .detach();
        }

        terminate_result
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
