//! Job management

use std::collections::VecDeque;
use std::fmt::Display;

use futures::FutureExt;

use crate::engine::ExecutionResult;
use crate::engine::error;
use crate::engine::processes;
use crate::engine::sys;
use crate::engine::trace_categories;
use crate::engine::traps;

pub(crate) type JobJoinHandle = compio::runtime::JoinHandle<Result<ExecutionResult, error::Error>>;
pub(crate) type JobResult = (Job, Result<ExecutionResult, error::Error>);

/// Manages the jobs that are currently managed by the shell.
pub struct JobManager {
    /// The jobs that are currently managed by the shell.
    pub jobs: Vec<Job>,
    /// 下一个要分配的 shell 内部 job ID.
    next_job_id: usize,
    /// 当前 job 的 ID.
    current_job_id: Option<usize>,
    /// previous job 的 ID.
    previous_job_id: Option<usize>,
}

impl Default for JobManager {
    fn default() -> Self {
        Self {
            jobs: Vec::new(),
            next_job_id: 1,
            current_job_id: None,
            previous_job_id: None,
        }
    }
}

/// Represents a task that is part of a job.
pub enum JobTask {
    /// An external process.
    External(processes::ChildProcess),
    /// An internal asynchronous task.
    Internal(JobJoinHandle),
}

impl JobTask {
    /// Returns whether the task is an external process.
    pub const fn is_external(&self) -> bool {
        matches!(self, Self::External(_))
    }

    /// Waits for the task to complete. Returns the task's execution result.
    pub async fn wait(&mut self) -> Result<ExecutionResult, error::Error> {
        match self {
            Self::External(process) => match process.wait().await? {
                processes::ProcessWaitResult::Completed(output) => Ok(output.into()),
                processes::ProcessWaitResult::Stopped => Ok(ExecutionResult::stopped()),
            },
            Self::Internal(handle) => handle
                .await
                .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))?,
        }
    }

    /// Polls the task for completion. Returns `Some(result)` if the task has completed,
    /// or `None` if it is still running. The result is the execution result of the task.
    /// Behaves in a best-effort manner; if an internal error occurs during polling,
    /// it will return `None`.
    fn poll(&mut self) -> Option<Result<ExecutionResult, error::Error>> {
        match self {
            Self::External(process) => {
                let check_result = process.poll();
                check_result.map(|polled_result| polled_result.map(|output| output.into()))
            }
            Self::Internal(handle) => {
                let checkable_handle = handle;
                checkable_handle.now_or_never().and_then(|r| r.ok())
            }
        }
    }
}

impl JobManager {
    /// Returns a new job manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a job to the job manager and marks it as the current job;
    /// returns an immutable reference to the job.
    ///
    /// # Arguments
    ///
    /// * `job` - The job to add.
    #[allow(
        clippy::missing_panics_doc,
        reason = "push() guarantees the vector length is >= 1"
    )]
    pub fn add_as_current(&mut self, mut job: Job) -> &Job {
        let id = self.next_job_id;
        self.next_job_id = self
            .next_job_id
            .checked_add(1)
            .expect("job id overflow while assigning new job");

        self.previous_job_id = self.current_job_id.filter(|id| self.job_exists(*id));
        self.current_job_id = Some(id);

        job.id = id;
        self.jobs.push(job);
        self.refresh_annotations();

        #[allow(clippy::unwrap_used, reason = "we just pushed an element")]
        self.jobs.last().unwrap()
    }

    /// Returns the current job, if there is one.
    pub fn current_job(&self) -> Option<&Job> {
        let id = self.current_job_id?;
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Returns a mutable reference to the current job, if there is one.
    pub fn current_job_mut(&mut self) -> Option<&mut Job> {
        let id = self.current_job_id?;
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// Returns the previous job, if there is one.
    pub fn prev_job(&self) -> Option<&Job> {
        let id = self.previous_job_id?;
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Returns a mutable reference to the previous job, if there is one.
    pub fn prev_job_mut(&mut self) -> Option<&mut Job> {
        let id = self.previous_job_id?;
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// Tries to resolve the given job specification to a job.
    ///
    /// # Arguments
    ///
    /// * `job_spec` - The job specification to resolve.
    pub fn resolve_job_spec(&mut self, job_spec: &str) -> Option<&mut Job> {
        let remainder = job_spec.strip_prefix('%')?;

        match remainder {
            "%" | "+" => self.current_job_mut(),
            "-" => self.prev_job_mut(),
            s if s.chars().all(char::is_numeric) => {
                let id = s.parse::<usize>().ok()?;
                self.jobs.iter_mut().find(|j| j.id == id)
            }
            _ => {
                log::warn!(target: trace_categories::UNIMPLEMENTED, "unimplemented: job spec naming command: '{job_spec}'");
                None
            }
        }
    }

    /// Waits for all managed jobs to complete.
    pub async fn wait_all(&mut self) -> Result<Vec<Job>, error::Error> {
        for job in &mut self.jobs {
            job.wait().await?;
        }

        Ok(self.sweep_completed_jobs())
    }

    /// Polls all managed jobs for completion.
    pub fn poll(&mut self) -> Result<Vec<JobResult>, error::Error> {
        let mut results = vec![];

        let mut i = 0;
        while i != self.jobs.len() {
            if let Some(result) = self.jobs[i].poll_done()? {
                let job = self.remove_job_at(i);
                results.push((job, result));
            } else if matches!(self.jobs[i].state, JobState::Done) {
                // TODO(jobs): This is a workaround to remove jobs that are done but for which we
                // don't know what happened.
                results.push((self.remove_job_at(i), Ok(ExecutionResult::success())));
            } else {
                i += 1;
            }
        }

        Ok(results)
    }

    fn sweep_completed_jobs(&mut self) -> Vec<Job> {
        let mut completed_jobs = vec![];

        let mut i = 0;
        while i != self.jobs.len() {
            if self.jobs[i].tasks.is_empty() {
                completed_jobs.push(self.remove_job_at(i));
            } else {
                i += 1;
            }
        }

        completed_jobs
    }

    fn remove_job_at(&mut self, index: usize) -> Job {
        let job = self.jobs.remove(index);
        if self.current_job_id == Some(job.id) {
            self.current_job_id = None;
        }
        if self.previous_job_id == Some(job.id) {
            self.previous_job_id = None;
        }
        self.refresh_annotations();
        job
    }

    fn refresh_annotations(&mut self) {
        if self.jobs.is_empty() {
            self.current_job_id = None;
            self.previous_job_id = None;
            return;
        }

        if self.current_job_id.is_none_or(|id| !self.job_exists(id)) {
            self.current_job_id = self.newest_job_id_except(None);
        }

        if self
            .previous_job_id
            .is_none_or(|id| Some(id) == self.current_job_id || !self.job_exists(id))
        {
            self.previous_job_id = self.newest_job_id_except(self.current_job_id);
        }

        for job in &mut self.jobs {
            job.annotation = if Some(job.id) == self.current_job_id {
                JobAnnotation::Current
            } else if Some(job.id) == self.previous_job_id {
                JobAnnotation::Previous
            } else {
                JobAnnotation::None
            };
        }
    }

    fn job_exists(&self, id: usize) -> bool {
        self.jobs.iter().any(|job| job.id == id)
    }

    fn newest_job_id_except(&self, excluded: Option<usize>) -> Option<usize> {
        self.jobs
            .iter()
            .filter(|job| Some(job.id) != excluded)
            .map(|job| job.id)
            .max()
    }
}

/// Represents the current execution state of a job.
#[derive(Clone)]
pub enum JobState {
    /// Unknown state.
    Unknown,
    /// The job is running.
    Running,
    /// The job is stopped.
    Stopped,
    /// The job has completed.
    Done,
}

impl Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "Unknown"),
            Self::Running => write!(f, "Running"),
            Self::Stopped => write!(f, "Stopped"),
            Self::Done => write!(f, "Done"),
        }
    }
}

/// Represents an annotation for a job.
#[derive(Clone)]
pub enum JobAnnotation {
    /// No annotation.
    None,
    /// The job is the current job.
    Current,
    /// The job is the previous job.
    Previous,
}

impl Display for JobAnnotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, ""),
            Self::Current => write!(f, "+"),
            Self::Previous => write!(f, "-"),
        }
    }
}

/// Encapsulates a set of processes managed by the shell as a single unit.
pub struct Job {
    /// The tasks that make up the job.
    tasks: VecDeque<JobTask>,

    /// If available, the process group ID of the job's processes.
    pgid: Option<sys::process::ProcessId>,

    /// The annotation of the job (e.g., current, previous).
    annotation: JobAnnotation,

    /// The shell-internal ID of the job.
    pub id: usize,

    /// The command line of the job.
    pub command_line: String,

    /// The current operational state of the job.
    pub state: JobState,
}

impl Display for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}]{:3}{}\t{}",
            self.id,
            self.annotation.to_string(),
            self.state,
            self.command_line
        )
    }
}

impl Job {
    /// Returns a new job object.
    ///
    /// # Arguments
    ///
    /// * `children` - The job's known child processes.
    /// * `command_line` - The command line of the job.
    /// * `state` - The current operational state of the job.
    pub(crate) fn new<I>(tasks: I, command_line: String, state: JobState) -> Self
    where
        I: IntoIterator<Item = JobTask>,
    {
        Self {
            id: 0,
            tasks: tasks.into_iter().collect(),
            pgid: None,
            annotation: JobAnnotation::None,
            command_line,
            state,
        }
    }

    /// Returns a pid-style string for the job.
    pub fn to_pid_style_string(&self) -> String {
        let display_pid = self
            .representative_pid()
            .map_or_else(|| String::from("<pid unknown>"), |pid| pid.to_string());
        std::format!("[{}]{}\t{}", self.id, self.annotation, display_pid)
    }

    /// Returns the annotation of the job.
    pub fn annotation(&self) -> JobAnnotation {
        self.annotation.clone()
    }

    /// Returns the command name of the job.
    pub fn command_name(&self) -> &str {
        self.command_line
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
    }

    /// Returns whether the job is the current job.
    pub const fn is_current(&self) -> bool {
        matches!(self.annotation, JobAnnotation::Current)
    }

    /// Returns whether the job is the previous job.
    pub const fn is_prev(&self) -> bool {
        matches!(self.annotation, JobAnnotation::Previous)
    }

    /// Polls whether the job has completed.
    pub fn poll_done(
        &mut self,
    ) -> Result<Option<Result<ExecutionResult, error::Error>>, error::Error> {
        let mut result: Option<Result<ExecutionResult, error::Error>> = None;

        log::debug!(target: trace_categories::JOBS, "Polling job {} for completion...", self.id);

        while !self.tasks.is_empty() {
            let task = &mut self.tasks[0];
            match task.poll() {
                Some(r) => {
                    self.tasks.remove(0);
                    result = Some(r);
                }
                None => {
                    return Ok(None);
                }
            }
        }

        log::debug!(target: trace_categories::JOBS, "Job {} has completed.", self.id);

        self.state = JobState::Done;

        Ok(result)
    }

    /// Waits for the job to complete.
    pub async fn wait(&mut self) -> Result<ExecutionResult, error::Error> {
        let mut result = ExecutionResult::success();

        while let Some(task) = self.tasks.back_mut() {
            let execution_result = task.wait().await?;
            if execution_result.exit_code == ExecutionResult::stopped().exit_code {
                self.state = JobState::Stopped;
                return Ok(execution_result);
            }

            result = execution_result;
            self.tasks.pop_back();
        }

        self.state = JobState::Done;

        Ok(result)
    }

    /// Moves the job to execute in the background.
    pub fn move_to_background(&mut self) -> Result<(), error::Error> {
        if !sys::signal::supports_job_signals() {
            return Err(error::ErrorKind::NotSupported("background job control").into());
        }

        if matches!(self.state, JobState::Stopped) {
            if let Some(pgid) = self.process_group_id() {
                sys::signal::continue_process(pgid)?;
                self.state = JobState::Running;
                Ok(())
            } else {
                Err(error::ErrorKind::FailedToSendSignal.into())
            }
        } else {
            error::unimp("move job to background")
        }
    }

    /// Moves the job to execute in the foreground.
    pub fn move_to_foreground(&mut self) -> Result<(), error::Error> {
        if !sys::terminal::supports_foreground_control() {
            return Err(error::ErrorKind::NotSupported("foreground job control").into());
        }

        if matches!(self.state, JobState::Stopped) {
            if let Some(pgid) = self.process_group_id() {
                sys::signal::continue_process(pgid)?;
                self.state = JobState::Running;
            } else {
                return Err(error::ErrorKind::FailedToSendSignal.into());
            }
        }

        if let Some(pgid) = self.process_group_id() {
            sys::terminal::move_to_foreground(pgid)?;
        }

        Ok(())
    }

    /// Kills the job.
    ///
    /// # Arguments
    ///
    /// * `signal` - The signal to send to the job.
    pub fn kill(&self, signal: traps::TrapSignal) -> Result<(), error::Error> {
        if let Some(pid) = self.process_group_id() {
            sys::signal::kill_process(pid, signal)
        } else {
            Err(error::ErrorKind::FailedToSendSignal.into())
        }
    }

    /// Tries to retrieve a "representative" pid for the job.
    pub fn representative_pid(&self) -> Option<sys::process::ProcessId> {
        for task in &self.tasks {
            match task {
                JobTask::External(p) => {
                    if let Some(pid) = p.pid() {
                        return Some(pid);
                    }
                }
                JobTask::Internal(_) => (),
            }
        }
        None
    }

    /// Tries to retrieve the process group ID (PGID) of the job.
    pub fn process_group_id(&self) -> Option<sys::process::ProcessId> {
        // TODO(jobs): Don't assume that the first PID is the PGID.
        self.pgid.or_else(|| self.representative_pid())
    }
}
