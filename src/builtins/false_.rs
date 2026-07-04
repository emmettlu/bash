use crate::engine::{ExecutionResult, builtins};

/// Return exit code 1.
pub(crate) struct FalseCommand {}

impl builtins::Command for FalseCommand {
    type Error = crate::engine::Error;

    fn new<I>(_args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {})
    }

    async fn execute(
        &self,
        _context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        Ok(ExecutionResult::general_error())
    }
}

impl builtins::SimpleCommand for FalseCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, crate::engine::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Returns a failure exit status.".into()),
            builtins::ContentType::ShortUsage => Ok("false".into()),
            builtins::ContentType::ShortDescription => Ok("false - fail".into()),
        }
    }

    fn execute<I: Iterator<Item = S>, S: AsRef<str>>(
        _context: crate::engine::ExecutionContext<'_>,
        _args: I,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        Ok(ExecutionResult::general_error())
    }
}
