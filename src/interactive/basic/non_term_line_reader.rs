use std::io::Write as _;

use crate::interactive::{ReadResult, input_backend::normalize_line_ending};

pub(crate) struct NonTermLineReader;

impl super::LineReader for NonTermLineReader {
    fn read_line(
        &self,
        prompt: Option<&str>,
        _completion_handler: impl FnMut(
            &str,
            usize,
        ) -> Result<
            crate::engine::completion::Completions,
            crate::interactive::ShellError,
        >,
    ) -> Result<crate::interactive::ReadResult, crate::interactive::ShellError> {
        if let Some(prompt) = prompt {
            eprint!("{prompt}");
            std::io::stderr().flush()?;
        }

        let mut input = String::new();
        let bytes_read = std::io::stdin().read_line(&mut input)?;
        normalize_line_ending(&mut input);

        if bytes_read == 0 {
            Ok(ReadResult::Eof)
        } else {
            Ok(ReadResult::Input(input))
        }
    }
}
