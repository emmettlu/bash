use std::io::IsTerminal;

use crate::core::Shell;

use crate::interactive::{
    InputBackend, ShellError, completion,
    input_backend::{InteractivePrompt, ReadResult},
};

use super::{non_term_line_reader, term_line_reader};

/// Represents a basic shell input backend capable of interactive usage, with primitive support
/// for completion and test-focused automation via pexpect and similar technologies.
#[derive(Default)]
pub struct BasicInputBackend;

impl InputBackend for BasicInputBackend {
    fn read_line(
        &mut self,
        shell: &crate::interactive::ShellRef<impl crate::core::ShellExtensions>,
        prompt: InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        if std::io::stdin().is_terminal() {
            self.read_line_via(shell, &term_line_reader::TermLineReader::new()?, &prompt)
        } else {
            self.read_line_via(shell, &non_term_line_reader::NonTermLineReader, &prompt)
        }
    }
}

impl BasicInputBackend {
    fn read_line_via<R: super::LineReader, SE: crate::core::ShellExtensions>(
        &self,
        shell_ref: &crate::interactive::ShellRef<SE>,
        reader: &R,
        prompt: &InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        let mut prompt_to_use = self.should_display_prompt().then_some(&prompt);
        let mut result = String::new();

        loop {
            match reader.read_line(prompt_to_use.map(|p| p.prompt.as_str()), |line, cursor| {
                let mut shell =
                    compio::runtime::Runtime::with_current(|rt| rt.block_on(shell_ref.lock()));

                Self::generate_completions(&mut shell, line, cursor)
            })? {
                ReadResult::Input(s) => {
                    result.push_str(s.as_str());

                    let shell =
                        compio::runtime::Runtime::with_current(|rt| rt.block_on(shell_ref.lock()));

                    if Self::is_valid_input(&shell, result.as_str()) {
                        break;
                    }

                    prompt_to_use = None;
                }
                ReadResult::BoundCommand(s) => {
                    result.push_str(s.as_str());
                    break;
                }
                ReadResult::Eof => {
                    if result.is_empty() {
                        return Ok(ReadResult::Eof);
                    }
                    break;
                }
                ReadResult::Interrupted => return Ok(ReadResult::Interrupted),
            }
        }

        Ok(ReadResult::Input(result))
    }

    #[expect(clippy::unused_self)]
    fn should_display_prompt(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn is_valid_input(shell: &Shell<impl crate::core::ShellExtensions>, input: &str) -> bool {
        match shell.parse_string(input.to_owned()) {
            // Incomplete tokenizing (unclosed quotes, etc.) - need more input
            Err(crate::parser::ParseError::Tokenizing { inner, position: _ })
                if inner.is_incomplete() =>
            {
                false
            }
            // Parse error at end of input - could be incomplete
            Err(crate::parser::ParseError::ParsingAtEndOfInput) => false,
            // Parse error at a specific position OR successful parse - complete
            _ => true,
        }
    }

    fn generate_completions(
        shell: &mut Shell<impl crate::core::ShellExtensions>,
        line: &str,
        cursor: usize,
    ) -> Result<crate::core::completion::Completions, ShellError> {
        compio::runtime::Runtime::with_current(|rt| {
            rt.block_on(Self::generate_completions_async(shell, line, cursor))
        })
    }

    async fn generate_completions_async(
        shell: &mut Shell<impl crate::core::ShellExtensions>,
        line: &str,
        cursor: usize,
    ) -> Result<crate::core::completion::Completions, ShellError> {
        Ok(completion::complete_async(shell, line, cursor).await)
    }
}
