//! Process management

use std::sync::{
    Arc, Mutex,
    mpsc::{self, Receiver, TryRecvError},
};

use crate::engine::{error, sys};

/// Tracks a child process being awaited.
pub struct ChildProcess {
    /// Receives the output produced by the background wait thread.
    output_rx: Option<Arc<Mutex<Receiver<std::io::Result<std::process::Output>>>>>,
    /// If available, the process ID of the child.
    pid: Option<sys::process::ProcessId>,
    /// If available, the process group ID of the child.
    pgid: Option<sys::process::ProcessId>,
}

impl ChildProcess {
    /// Wraps a child process and starts waiting for it on a blocking thread.
    pub fn new(
        child: sys::process::Child,
        pid: Option<sys::process::ProcessId>,
        pgid: Option<sys::process::ProcessId>,
    ) -> Self {
        let (output_tx, output_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = output_tx.send(child.wait_with_output());
        });

        Self {
            output_rx: Some(Arc::new(Mutex::new(output_rx))),
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
        let Some(output_rx) = self.output_rx.take() else {
            return Err(error::ErrorKind::InternalError("process already waited".into()).into());
        };

        let output = compio::runtime::spawn_blocking(move || {
            output_rx
                .lock()
                .map_err(|err| std::io::Error::other(format!("process wait lock poisoned: {err}")))?
                .recv()
                .map_err(|err| {
                    std::io::Error::other(format!("process wait thread failed: {err}"))
                })?
        })
        .await
        .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))??;

        Ok(ProcessWaitResult::Completed(output))
    }

    /// Polls the process for completion without blocking.
    pub(crate) fn poll(&mut self) -> Option<Result<std::process::Output, error::Error>> {
        let output_rx = self.output_rx.as_ref()?;
        let poll_result = output_rx
            .lock()
            .map_err(|err| {
                error::Error::from(error::ErrorKind::ThreadingError(format!(
                    "process wait lock poisoned: {err}"
                )))
            })
            .and_then(|receiver| match receiver.try_recv() {
                Ok(output) => Ok(Some(output)),
                Err(TryRecvError::Empty) => Ok(None),
                Err(TryRecvError::Disconnected) => Err(error::ErrorKind::ThreadingError(
                    "process wait thread disconnected".into(),
                )
                .into()),
            });

        match poll_result {
            Ok(Some(output)) => {
                self.output_rx = None;
                Some(output.map_err(Into::into))
            }
            Ok(None) => None,
            Err(err) => {
                self.output_rx = None;
                Some(Err(err))
            }
        }
    }
}

/// Represents the result of waiting for an executing process.
pub enum ProcessWaitResult {
    /// The process completed.
    Completed(std::process::Output),
    /// The process stopped and has not yet completed.
    Stopped,
}
