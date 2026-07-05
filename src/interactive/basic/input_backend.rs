use std::io::IsTerminal;

use crate::engine::Shell;

use crate::interactive::{
    InputBackend, ShellError, completion,
    input_backend::{InteractivePrompt, ReadResult},
};

use super::{non_term_line_reader, term_line_reader};

/// Represents a basic shell input backend capable of interactive usage, with primitive support
/// for completion and test-focused automation via pexpect and similar technologies.
#[derive(Default)]
pub struct BasicInputBackend {
    pending_prompt_newline: bool,
}

impl InputBackend for BasicInputBackend {
    fn read_line(
        &mut self,
        shell: &mut Shell,
        prompt: InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        let is_terminal = std::io::stdin().is_terminal();
        let mut prompt = prompt;
        if is_terminal && self.pending_prompt_newline {
            prompt.prompt.insert(0, '\n');
            self.pending_prompt_newline = false;
        }

        let result = if is_terminal {
            self.read_line_via(shell, &term_line_reader::TermLineReader::new()?, &prompt)?
        } else {
            self.read_line_via(shell, &non_term_line_reader::NonTermLineReader, &prompt)?
        };

        if is_terminal
            && let ReadResult::Input(line) = &result
            && line.trim().is_empty()
        {
            self.pending_prompt_newline = true;
        }

        Ok(result)
    }
}

impl BasicInputBackend {
    fn read_line_via<R: super::LineReader>(
        &self,
        shell: &mut Shell,
        reader: &R,
        prompt: &InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        let should_display_prompt = self.should_display_prompt();
        let mut prompt_to_use = should_display_prompt.then_some(prompt.prompt.as_str());
        let mut result = String::new();

        let history_entries = Self::history_entries(shell);

        loop {
            match reader.read_line(prompt_to_use, &history_entries, |line, cursor| {
                Self::generate_completions(shell, line, cursor)
            })? {
                ReadResult::Input(s) => {
                    result.push_str(s.as_str());

                    if Self::is_valid_input(shell, result.as_str()) {
                        break;
                    }

                    prompt_to_use =
                        should_display_prompt.then_some(prompt.continuation_prompt.as_str());
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

    fn is_valid_input(shell: &Shell, input: &str) -> bool {
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

    fn history_entries(shell: &Shell) -> Vec<String> {
        shell
            .history()
            .map(|history| {
                history
                    .iter()
                    .map(|item| item.command_line.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn generate_completions(
        shell: &mut Shell,
        line: &str,
        cursor: usize,
    ) -> Result<crate::engine::completion::Completions, ShellError> {
        compio::runtime::Runtime::with_current(|rt| {
            rt.block_on(Self::generate_completions_async(shell, line, cursor))
        })
    }

    async fn generate_completions_async(
        shell: &mut Shell,
        line: &str,
        cursor: usize,
    ) -> Result<crate::engine::completion::Completions, ShellError> {
        Ok(completion::complete_async(shell, line, cursor).await)
    }
}
