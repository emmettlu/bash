use std::io::Write;

use crate::engine::{ExecutionResult, builtins, timing};

/// Report on usage time.
pub(crate) struct TimesCommand {}

impl builtins::Command for TimesCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        if args.next_arg().is_some() {
            return Err("times: too many arguments".into());
        }
        Ok(Self {})
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        let (self_user, self_system) =
            crate::engine::sys::resource::get_self_user_and_system_time()?;
        writeln!(
            context.stdout(),
            "{} {}",
            timing::format_duration_non_posixly(&self_user),
            timing::format_duration_non_posixly(&self_system),
        )?;

        let (children_user, children_system) =
            crate::engine::sys::resource::get_children_user_and_system_time()?;
        writeln!(
            context.stdout(),
            "{} {}",
            timing::format_duration_non_posixly(&children_user),
            timing::format_duration_non_posixly(&children_system),
        )?;

        Ok(ExecutionResult::success())
    }
}
