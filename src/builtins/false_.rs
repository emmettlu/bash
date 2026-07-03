use crate::engine::{ExecutionResult, builtins};

/// Return exit code 1.
pub(crate) struct FalseCommand {}

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

    fn execute<SE: crate::engine::ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        _context: crate::engine::ExecutionContext<'_, SE>,
        _args: I,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        Ok(ExecutionResult::general_error())
    }
}
