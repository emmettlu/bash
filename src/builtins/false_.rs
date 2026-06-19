use crate::core::{ExecutionResult, builtins, error};

/// Return exit code 1.
pub(crate) struct FalseCommand {}

impl builtins::SimpleCommand for FalseCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, crate::core::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Returns a failure exit status.".into()),
            builtins::ContentType::ShortUsage => Ok("false".into()),
            builtins::ContentType::ShortDescription => Ok("false - fail".into()),
            builtins::ContentType::ManPage => error::unimp("man page not yet implemented"),
        }
    }

    fn execute<SE: crate::core::ShellExtensions, I: Iterator<Item = S>, S: AsRef<str>>(
        _context: crate::core::ExecutionContext<'_, SE>,
        _args: I,
    ) -> Result<ExecutionResult, crate::core::Error> {
        Ok(ExecutionResult::general_error())
    }
}
