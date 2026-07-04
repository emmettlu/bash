use std::io::Write;

use crate::engine::{ExecutionResult, builtins, error};

/// Wait for jobs to terminate.
pub(crate) struct WaitCommand {
    /// Wait for specified job to terminate (instead of change status).
    wait_for_terminate: bool,

    /// Wait for a single job to change status; if jobs are specified, waits for
    /// the first to change status, and otherwise waits for the next change.
    wait_for_first_or_next: bool,

    /// Name of variable to receive the job ID of the job whose status is indicated.
    variable_to_receive_id: Option<String>,

    /// Process IDs or job specs to wait for.
    ids: Vec<String>,
}

impl builtins::Command for WaitCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            wait_for_terminate: false,
            wait_for_first_or_next: false,
            variable_to_receive_id: None,
            ids: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);
        command.ids = parse_wait_args(&mut args, &mut command)?;
        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        if self.wait_for_terminate {
            return error::unimp("wait -f");
        }
        if self.wait_for_first_or_next {
            return error::unimp("wait -n");
        }
        if self.variable_to_receive_id.is_some() {
            return error::unimp("wait -p");
        }

        let mut result = ExecutionResult::success();

        if !self.ids.is_empty() {
            for id in &self.ids {
                if id.starts_with('%') {
                    // It's a job spec.
                    if let Some(job) = context.shell.jobs_mut().resolve_job_spec(id) {
                        job.wait().await?;
                    } else {
                        writeln!(
                            context.stderr(),
                            "{}: no such job: {}",
                            context.command_name,
                            id
                        )?;

                        result = ExecutionResult::general_error();
                    }
                } else {
                    // It's a process ID.
                    return error::unimp("wait with process IDs");
                }
            }
        } else {
            // Wait for all jobs.
            let jobs = context.shell.jobs_mut().wait_all().await?;

            if context.shell.options().enable_job_control {
                for job in jobs {
                    writeln!(context.stdout(), "{job}")?;
                }
            }
        }

        Ok(result)
    }
}

fn parse_wait_args(
    args: &mut builtins::BuiltinArgs,
    command: &mut WaitCommand,
) -> Result<Vec<String>, String> {
    args.parse_short_options(|args, flag, flags, rest_start| match flag {
        'f' => {
            command.wait_for_terminate = true;
            Ok(builtins::ShortOptionDisposition::Continue)
        }
        'n' => {
            command.wait_for_first_or_next = true;
            Ok(builtins::ShortOptionDisposition::Continue)
        }
        'p' => {
            command.variable_to_receive_id =
                Some(args.option_value(flags, rest_start, "wait: -p")?);
            Ok(builtins::ShortOptionDisposition::StopParsingArgument)
        }
        _ => Err(format!("wait: -{flag}: invalid option")),
    })
}
