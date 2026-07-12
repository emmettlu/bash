use std::{
    future::Future,
    io::{IsTerminal, Write},
};

use crate::interactive::{
    InputBackend, ShellError,
    input_backend::{InteractivePrompt, ReadResult, is_complete_input, normalize_line_ending},
};

/// Represents a minimal shell input backend, capable of taking commands from standard input.
#[derive(Default)]
pub struct MinimalInputBackend;

impl InputBackend for MinimalInputBackend {
    async fn read_line(
        &mut self,
        shell: &mut crate::engine::Shell,
        prompt: InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        let continue_incomplete_input = !std::io::stdin().is_terminal();
        self.display_prompt(&prompt)?;

        Self::read_complete_input(shell, continue_incomplete_input, Self::read_input_line).await
    }
}

impl MinimalInputBackend {
    #[expect(clippy::unused_self)]
    fn should_display_prompt(&self) -> bool {
        std::io::stdin().is_terminal()
    }

    fn display_prompt(&self, prompt: &InteractivePrompt) -> Result<(), ShellError> {
        if self.should_display_prompt() {
            eprint!("{}", prompt.prompt);
            std::io::stderr().flush()?;
        }

        Ok(())
    }

    async fn read_complete_input<F, Fut>(
        shell: &crate::engine::Shell,
        continue_incomplete_input: bool,
        mut read_line: F,
    ) -> Result<ReadResult, ShellError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<ReadResult, ShellError>>,
    {
        let mut input = String::new();

        loop {
            match read_line().await? {
                ReadResult::Input(line) | ReadResult::BoundCommand(line) => {
                    input.push_str(&line);
                    if !continue_incomplete_input || is_complete_input(shell, &input) {
                        return Ok(ReadResult::Input(input));
                    }
                }
                ReadResult::Eof if input.is_empty() => return Ok(ReadResult::Eof),
                ReadResult::Eof => return Ok(ReadResult::Input(input)),
                ReadResult::Interrupted => return Ok(ReadResult::Interrupted),
            }
        }
    }

    async fn read_input_line() -> Result<ReadResult, ShellError> {
        compio::runtime::spawn_blocking(|| {
            let mut input = String::new();
            let bytes_read = std::io::stdin().read_line(&mut input)?;
            normalize_line_ending(&mut input);

            if bytes_read == 0 {
                Ok(ReadResult::Eof)
            } else {
                Ok(ReadResult::Input(input))
            }
        })
        .await
        .map_err(|err| std::io::Error::other(format!("input worker panicked: {err:?}")))?
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, future::ready};

    use super::*;

    #[compio::test]
    async fn non_terminal_input_collects_incomplete_lines() -> anyhow::Result<()> {
        let shell = crate::engine::Shell::builder().build().await?;
        let mut reads = VecDeque::from([
            ReadResult::Input("if true; then\n".into()),
            ReadResult::Input("echo continued; fi\n".into()),
        ]);

        let result = MinimalInputBackend::read_complete_input(&shell, true, || {
            ready(Ok(reads.pop_front().expect("测试输入不足")))
        })
        .await?;

        assert_eq!(
            result,
            ReadResult::Input("if true; then\necho continued; fi\n".into())
        );
        assert!(reads.is_empty());
        Ok(())
    }

    #[compio::test]
    async fn empty_line_is_not_eof() -> anyhow::Result<()> {
        let shell = crate::engine::Shell::builder().build().await?;
        let mut reads = VecDeque::from([ReadResult::Input("\n".into()), ReadResult::Eof]);

        let result = MinimalInputBackend::read_complete_input(&shell, true, || {
            ready(Ok(reads.pop_front().expect("测试输入不足")))
        })
        .await?;

        assert_eq!(result, ReadResult::Input("\n".into()));
        assert_eq!(reads, VecDeque::from([ReadResult::Eof]));
        Ok(())
    }

    #[compio::test]
    async fn eof_returns_buffered_incomplete_input_once() -> anyhow::Result<()> {
        let shell = crate::engine::Shell::builder().build().await?;
        let mut reads = VecDeque::from([
            ReadResult::Input("echo 'unterminated\n".into()),
            ReadResult::Eof,
        ]);

        let result = MinimalInputBackend::read_complete_input(&shell, true, || {
            ready(Ok(reads.pop_front().expect("测试输入不足")))
        })
        .await?;

        assert_eq!(result, ReadResult::Input("echo 'unterminated\n".into()));
        Ok(())
    }
}
