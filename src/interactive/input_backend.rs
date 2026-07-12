use crate::interactive::ShellError;

/// Represents an input backend for reading lines of input.
pub trait InputBackend: Send {
    /// Reads a line of input, using the given prompt.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell instance for which input is being read.
    /// * `prompt` - The prompt to display to the user.
    fn read_line<'a>(
        &'a mut self,
        shell: &'a mut crate::engine::Shell,
        prompt: InteractivePrompt,
    ) -> impl Future<Output = Result<ReadResult, ShellError>> + 'a;

    /// Returns the current contents of the read buffer and the current cursor
    /// position within the buffer; None is returned if the read buffer is
    /// empty or cannot be read by this implementation.
    fn get_read_buffer(&self) -> Option<(String, usize)> {
        None
    }

    /// Updates the read buffer with the given string and cursor. Considered a
    /// no-op if the implementation does not support updating read buffers.
    fn set_read_buffer(&mut self, _buffer: String, _cursor: usize) {
        // No-op by default.
    }
}

/// 将 Windows 控制台输入的 CRLF 行尾规范化为 shell 内部使用的 LF。
pub(crate) fn normalize_line_ending(input: &mut String) {
    if input.ends_with("\r\n") {
        input.truncate(input.len() - 2);
        input.push('\n');
    } else if input.ends_with('\r') {
        input.pop();
        input.push('\n');
    }
}

/// 判断输入是否已经形成完整 shell 语法。
pub(crate) fn is_complete_input(shell: &crate::engine::Shell, input: &str) -> bool {
    match shell.parse_string(input.to_owned()) {
        Err(crate::parser::ParseError::Tokenizing { inner, position: _ })
            if inner.is_incomplete() =>
        {
            false
        }
        Err(
            crate::parser::ParseError::ParsingAtEndOfInput
            | crate::parser::ParseError::ParsingAtEndOfInputWithExpected { .. },
        ) => false,
        _ => true,
    }
}

/// Result of a read operation.
#[derive(Debug, Eq, PartialEq)]
pub enum ReadResult {
    /// The user entered a line of input.
    Input(String),
    /// A bound key sequence yielded a registered command.
    BoundCommand(String),
    /// End of input was reached.
    Eof,
    /// The user interrupted the input operation.
    Interrupted,
}

/// Represents an interactive prompt.
pub struct InteractivePrompt {
    /// Prompt to display.
    pub prompt: String,
    /// Alternate-side prompt (typically right) to display.
    pub alt_side_prompt: String,
    /// Prompt to display on a continuation line of input.
    pub continuation_prompt: String,
}

#[cfg(test)]
mod tests {
    use super::is_complete_input;

    #[compio::test]
    async fn complete_input_distinguishes_continuations_from_errors() -> anyhow::Result<()> {
        let shell = crate::engine::Shell::builder().build().await?;

        assert!(!is_complete_input(&shell, "echo 'unterminated\n"));
        assert!(!is_complete_input(&shell, "if true; then\n"));
        assert!(is_complete_input(&shell, "echo complete\n"));
        assert!(is_complete_input(&shell, "if ; then\n"));

        Ok(())
    }
}
