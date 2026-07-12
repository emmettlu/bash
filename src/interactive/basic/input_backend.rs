use std::{
    io::IsTerminal,
    sync::{Arc, mpsc},
};

use futures::{FutureExt as _, StreamExt as _};

use crate::engine::Shell;

use crate::interactive::{
    InputBackend, ShellError, completion,
    input_backend::{InteractivePrompt, ReadResult, is_complete_input},
};

use super::{LineReader as _, non_term_line_reader, term_line_reader};

/// Represents a basic shell input backend capable of interactive usage, with primitive support
/// for completion and test-focused automation via pexpect and similar technologies.
#[derive(Default)]
pub struct BasicInputBackend {
    pending_prompt_newline: bool,
    history_entries: Arc<Vec<String>>,
    history_revision: Option<u64>,
    history_non_append_revision: Option<u64>,
}

impl InputBackend for BasicInputBackend {
    async fn read_line(
        &mut self,
        shell: &mut Shell,
        prompt: InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        self.read_line_async(shell, prompt).await
    }
}

impl BasicInputBackend {
    async fn read_line_async(
        &mut self,
        shell: &mut Shell,
        mut prompt: InteractivePrompt,
    ) -> Result<ReadResult, ShellError> {
        let is_terminal = std::io::stdin().is_terminal();
        if is_terminal && self.pending_prompt_newline {
            prompt.prompt.insert(0, '\n');
            self.pending_prompt_newline = false;
        }

        self.sync_history_entries(shell);
        let mut prompt_to_use = is_terminal.then_some(prompt.prompt.as_str());
        let mut input = String::new();

        loop {
            let read_result = if is_terminal {
                Self::read_terminal_line(
                    shell,
                    prompt_to_use.map(str::to_owned),
                    Arc::clone(&self.history_entries),
                )
                .await?
            } else {
                Self::read_non_terminal_line(prompt_to_use.map(str::to_owned)).await?
            };

            match read_result {
                ReadResult::Input(line) => {
                    input.push_str(&line);
                    if is_complete_input(shell, &input) {
                        break;
                    }

                    prompt_to_use = is_terminal.then_some(prompt.continuation_prompt.as_str());
                }
                ReadResult::BoundCommand(command) => {
                    input.push_str(&command);
                    break;
                }
                ReadResult::Eof if input.is_empty() => return Ok(ReadResult::Eof),
                ReadResult::Eof => break,
                ReadResult::Interrupted => return Ok(ReadResult::Interrupted),
            }
        }

        let result = ReadResult::Input(input);
        if is_terminal
            && let ReadResult::Input(line) = &result
            && line.trim().is_empty()
        {
            self.pending_prompt_newline = true;
        }

        Ok(result)
    }

    async fn read_non_terminal_line(prompt: Option<String>) -> Result<ReadResult, ShellError> {
        compio::runtime::spawn_blocking(move || {
            non_term_line_reader::NonTermLineReader.read_line(prompt.as_deref(), &[], |_, _| {
                unreachable!("非终端输入不会请求补全")
            })
        })
        .await
        .map_err(|err| std::io::Error::other(format!("input worker panicked: {err:?}")))?
    }

    async fn read_terminal_line(
        shell: &mut Shell,
        prompt: Option<String>,
        history_entries: Arc<Vec<String>>,
    ) -> Result<ReadResult, ShellError> {
        let (completion_tx, mut completion_rx) = futures::channel::mpsc::unbounded();
        let read_task = compio::runtime::spawn_blocking(move || {
            let reader = term_line_reader::TermLineReader::new()?;
            reader.read_line(prompt.as_deref(), &history_entries, |line, cursor| {
                let (response_tx, response_rx) = mpsc::sync_channel(1);
                completion_tx
                    .unbounded_send(CompletionRequest {
                        line: line.to_owned(),
                        cursor,
                        response_tx,
                    })
                    .map_err(|_| std::io::Error::other("completion worker stopped"))?;
                Ok(response_rx
                    .recv()
                    .map_err(|_| std::io::Error::other("completion response was dropped"))?)
            })
        })
        .fuse();
        futures::pin_mut!(read_task);

        loop {
            let request = completion_rx.next().fuse();
            futures::pin_mut!(request);

            futures::select! {
                result = read_task => {
                    return result.map_err(|err| {
                        std::io::Error::other(format!("input worker panicked: {err:?}"))
                    })?;
                }
                request = request => {
                    let Some(request) = request else {
                        return read_task.await.map_err(|err| {
                            std::io::Error::other(format!("input worker panicked: {err:?}"))
                        })?;
                    };
                    let completions = completion::complete_async(
                        shell,
                        &request.line,
                        request.cursor,
                    )
                    .await;
                    let _ = request.response_tx.send(completions);
                }
            }
        }
    }

    fn sync_history_entries(&mut self, shell: &Shell) {
        let Some(history) = shell.history() else {
            Arc::make_mut(&mut self.history_entries).clear();
            self.history_revision = None;
            self.history_non_append_revision = None;
            return;
        };

        if self.history_revision == Some(history.revision()) {
            return;
        }

        if self.history_non_append_revision == Some(history.non_append_revision())
            && history.count() >= self.history_entries.len()
        {
            let entry_count = self.history_entries.len();
            Arc::make_mut(&mut self.history_entries).extend(
                history
                    .iter()
                    .skip(entry_count)
                    .map(|item| item.command_line.clone()),
            );
        } else {
            self.history_entries = Arc::new(
                history
                    .iter()
                    .map(|item| item.command_line.clone())
                    .collect(),
            );
        }

        self.history_revision = Some(history.revision());
        self.history_non_append_revision = Some(history.non_append_revision());
    }
}

struct CompletionRequest {
    line: String,
    cursor: usize,
    response_tx: mpsc::SyncSender<crate::engine::completion::Completions>,
}

#[cfg(test)]
mod tests {
    use super::is_complete_input;

    #[compio::test]
    async fn expected_tokens_at_end_request_continuation() -> anyhow::Result<()> {
        let shell = crate::engine::Shell::builder().build().await?;
        let input = "echo hello &&";

        assert!(matches!(
            shell.parse_string(input),
            Err(crate::parser::ParseError::ParsingAtEndOfInputWithExpected { .. })
        ));
        assert!(!is_complete_input(&shell, input));

        Ok(())
    }
}
