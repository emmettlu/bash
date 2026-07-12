use itertools::Itertools;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::engine::{ErrorKind, builtins, env, error, variables};

use std::io::{Read, Write};

/// Exit code returned when `read` times out.
/// This is 128 + SIGALRM (14) = 142, matching bash behavior.
const TIMEOUT_EXIT_CODE: u8 = 142;

/// ASCII control character for Ctrl+C (ETX - End of Text).
const CTRL_C: char = '\x03';
/// ASCII control character for Ctrl+D (EOT - End of Transmission).
const CTRL_D: char = '\x04';
/// Backslash character used for escape processing.
const BACKSLASH: char = '\\';
/// Default line delimiter (newline).
const DEFAULT_DELIMITER: char = '\n';
/// NUL character used as delimiter when `-d ''` is specified.
const NUL_DELIMITER: char = '\0';

/// Parse standard input.
pub(crate) struct ReadCommand {
    /// Optionally, name of an array variable to receive read words
    /// of input.
    array_variable: Option<String>,

    /// Optionally, a delimiter to use other than a newline character.
    delimiter: Option<String>,

    /// Use readline-like input.
    use_readline: bool,

    /// Provide text to use as initial input for readline.
    initial_text: Option<String>,

    /// Read only the first N characters or until a specified
    /// delimiter is reached, whichever happens first.
    return_after_n_chars: Option<usize>,

    /// Read exactly N characters, ignoring any specified delimiter.
    return_after_n_chars_no_delimiter: Option<usize>,

    /// Prompt to display before reading.
    prompt: Option<String>,

    /// Read input in raw mode; no escape sequences.
    raw_mode: bool,

    /// Do not echo input.
    silent: bool,

    /// Specify timeout in seconds; fail if the timeout elapses before
    /// input is completed.
    timeout_in_seconds: Option<f64>,

    /// File descriptor to read from instead of stdin.
    fd_num_to_read: Option<u8>,

    /// Optionally, names of variables to receive read input.
    variable_names: Vec<String>,
}

impl builtins::Command for ReadCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            array_variable: None,
            delimiter: None,
            use_readline: false,
            initial_text: None,
            return_after_n_chars: None,
            return_after_n_chars_no_delimiter: None,
            prompt: None,
            raw_mode: false,
            silent: false,
            timeout_in_seconds: None,
            fd_num_to_read: None,
            variable_names: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);

        command.variable_names =
            args.parse_short_options(|args, flag, flags, rest_start| match flag {
                'e' => {
                    command.use_readline = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                'r' => {
                    command.raw_mode = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                's' => {
                    command.silent = true;
                    Ok(builtins::ShortOptionDisposition::Continue)
                }
                'a' => {
                    command.array_variable = Some(args.option_value(flags, rest_start, "-a")?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                'd' => {
                    command.delimiter = Some(args.option_value(flags, rest_start, "-d")?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                'i' => {
                    command.initial_text = Some(args.option_value(flags, rest_start, "-i")?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                'n' => {
                    let value = args.option_value(flags, rest_start, "-n")?;
                    command.return_after_n_chars = Some(parse_usize_option("-n", &value)?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                'N' => {
                    let value = args.option_value(flags, rest_start, "-N")?;
                    command.return_after_n_chars_no_delimiter =
                        Some(parse_usize_option("-N", &value)?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                'p' => {
                    command.prompt = Some(args.option_value(flags, rest_start, "-p")?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                't' => {
                    let value = args.option_value(flags, rest_start, "-t")?;
                    command.timeout_in_seconds = Some(parse_timeout_option(&value)?);
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                'u' => {
                    let value = args.option_value(flags, rest_start, "-u")?;
                    command.fd_num_to_read = Some(
                        value
                            .parse()
                            .map_err(|_| format!("-u: invalid file descriptor: {value}"))?,
                    );
                    Ok(builtins::ShortOptionDisposition::StopParsingArgument)
                }
                _ => Err(format!("-{flag}: invalid option")),
            })?;

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.use_readline {
            return error::unimp("read -e");
        }
        if self.initial_text.is_some() {
            return error::unimp("read -i");
        }

        // Validate timeout value if provided.
        if let Some(result) = self.validate_timeout(&context)? {
            return Ok(result);
        }

        // Find the input stream to use.
        let fd_num = self.fd_num_to_read.map_or(
            crate::engine::openfiles::OpenFiles::STDIN_FD,
            crate::engine::ShellFd::from,
        );

        // Retrieve the file.
        let input_stream = context
            .try_clone_fd(fd_num)?
            .ok_or_else(|| ErrorKind::BadFileDescriptor(fd_num))?;

        // Retrieve effective value of IFS for splitting.
        // We convert to owned String to release the borrow before the mutable borrow
        // needed for variable assignment.
        let ifs = context.shell.ifs().into_owned();

        // Convert timeout to Duration.
        let timeout = self.timeout_in_seconds.map(Duration::from_secs_f64);

        // 将完整读取循环移到阻塞线程, worker 仅返回 owned 读取结果。
        let worker = ReadWorker::from(self);
        let stderr = context.stderr();
        let read_result = compio::runtime::spawn_blocking(move || {
            worker.read_line(input_stream, stderr, timeout)
        })
        .await
        .map_err(|err| ErrorKind::ThreadingError(err.to_string()))??;

        // Determine whether to skip IFS splitting (for -N option).
        let skip_ifs_splitting = self.return_after_n_chars_no_delimiter.is_some();

        // Extract the input line and determine exit code based on result.
        let (input_line, result) = match &read_result {
            ReadResult::Line(line) => (
                Some(line.clone()),
                crate::engine::ExecutionResult::success(),
            ),
            ReadResult::Eof(Some(line)) => (
                Some(line.clone()),
                crate::engine::ExecutionResult::general_error(),
            ),
            ReadResult::Eof(None) | ReadResult::Interrupted | ReadResult::InputNotReady => {
                (None, crate::engine::ExecutionResult::general_error())
            }
            ReadResult::TimedOut(partial) => (
                partial.clone(),
                crate::engine::ExecutionResult::new(TIMEOUT_EXIT_CODE),
            ),
            ReadResult::InputReady => (None, crate::engine::ExecutionResult::success()),
        };

        // Assign input to variables based on options.
        assign_input_to_variables(
            context.shell,
            input_line.as_deref(),
            &ifs,
            skip_ifs_splitting,
            self.array_variable.as_deref(),
            &self.variable_names,
        )?;

        Ok(result)
    }
}

fn parse_usize_option(option: &str, value: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("{option}: invalid count: {value}"))
}

fn parse_timeout_option(value: &str) -> Result<f64, String> {
    let timeout = value
        .parse::<f64>()
        .map_err(|_| format!("-t: invalid timeout: {value}"))?;
    if timeout.is_finite() {
        Ok(timeout)
    } else {
        Err(format!("-t: invalid timeout: {value}"))
    }
}

/// Assigns read input to shell variables based on the specified options.
///
/// This handles three modes:
/// - Array mode (`-a`): Split input by IFS and assign to array elements
/// - Named variables: Split input by IFS and assign to each variable, with remainder to last
/// - Default (`REPLY`): Assign entire input line to the `REPLY` variable
fn assign_input_to_variables(
    shell: &mut crate::engine::Shell,
    input_line: Option<&str>,
    ifs: &str,
    skip_ifs_splitting: bool,
    array_variable: Option<&str>,
    variable_names: &[String],
) -> Result<(), crate::engine::Error> {
    if let Some(array_variable) = array_variable {
        let literal_fields = build_array_fields(input_line, ifs, skip_ifs_splitting);
        shell.env_mut().update_or_add(
            array_variable,
            variables::ShellValueLiteral::Array(variables::ArrayLiteral(literal_fields)),
            |_| Ok(()),
            env::EnvironmentLookup::Anywhere,
            env::EnvironmentScope::Global,
        )?;
    } else if !variable_names.is_empty() {
        assign_to_named_variables(shell, input_line, ifs, skip_ifs_splitting, variable_names)?;
    } else {
        shell.env_mut().update_or_add(
            "REPLY",
            variables::ShellValueLiteral::Scalar(input_line.unwrap_or_default().to_owned()),
            |_| Ok(()),
            env::EnvironmentLookup::Anywhere,
            env::EnvironmentScope::Global,
        )?;
    }
    Ok(())
}

/// Assigns split fields to named variables.
///
/// Fields are assigned one per variable, with any remaining fields joined by space
/// and assigned to the last variable. If there are more variables than fields,
/// the extra variables are set to empty strings.
fn assign_to_named_variables(
    shell: &mut crate::engine::Shell,
    input_line: Option<&str>,
    ifs: &str,
    skip_ifs_splitting: bool,
    variable_names: &[String],
) -> Result<(), crate::engine::Error> {
    let mut fields =
        build_variable_fields(input_line, ifs, skip_ifs_splitting, variable_names.len());

    for (i, name) in variable_names.iter().enumerate() {
        let is_last = i == variable_names.len() - 1;

        let value = if fields.is_empty() {
            String::new()
        } else if is_last {
            // Last variable gets all remaining fields joined by space.
            std::mem::take(&mut fields).into_iter().join(" ")
        } else {
            fields.pop_front().unwrap_or_default()
        };

        shell.env_mut().update_or_add(
            name,
            variables::ShellValueLiteral::Scalar(value),
            |_| Ok(()),
            env::EnvironmentLookup::Anywhere,
            env::EnvironmentScope::Global,
        )?;

        if is_last {
            break;
        }
    }
    Ok(())
}

/// Builds array field values from input, optionally splitting by IFS.
fn build_array_fields(
    input_line: Option<&str>,
    ifs: &str,
    skip_ifs_splitting: bool,
) -> Vec<(Option<String>, String)> {
    match input_line {
        Some(line) if skip_ifs_splitting => {
            // With -N, don't split - put entire input as single element.
            vec![(None, line.to_string())]
        }
        Some(line) => {
            let fields: VecDeque<_> = split_line_by_ifs(ifs, line, None /* max_fields */);
            fields.into_iter().map(|f| (None, f)).collect()
        }
        None => vec![],
    }
}

/// Builds field values from input for assignment to named variables.
fn build_variable_fields(
    input_line: Option<&str>,
    ifs: &str,
    skip_ifs_splitting: bool,
    num_variables: usize,
) -> VecDeque<String> {
    match input_line {
        Some(line) if skip_ifs_splitting => {
            // With -N, don't split - put entire input in first variable.
            let mut fields = VecDeque::new();
            fields.push_back(line.to_string());
            fields
        }
        Some(line) => split_line_by_ifs(ifs, line, Some(num_variables)),
        None => VecDeque::new(),
    }
}

/// Result of a `read` operation.
///
/// This enum clearly represents all possible outcomes of `read_line()`,
/// making the contract with callers explicit.
enum ReadResult {
    /// Successfully read a complete line (delimiter or char limit reached).
    Line(String),
    /// Reached end of input. Contains any partial content read before EOF.
    Eof(Option<String>),
    /// Input was interrupted (e.g., Ctrl+C). No content is returned.
    Interrupted,
    /// The operation timed out. Contains any partial content read before timeout.
    TimedOut(Option<String>),
    /// For `-t 0`: input is immediately available (exit 0).
    InputReady,
    /// For `-t 0`: no input immediately available (exit 1).
    InputNotReady,
}

/// Helper struct that encapsulates the state for reading input character by character.
///
/// This separates the concerns of character-level I/O with timeout handling from the
/// higher-level logic of line building and escape processing.
struct InputReader {
    /// The input source.
    input: crate::engine::openfiles::OpenFile,
    /// Optional deadline for timeout.
    deadline: Option<Instant>,
    /// UTF-8 字符读取缓冲区。
    buffer: [u8; 4],
    /// Terminal mode guard - kept alive for RAII cleanup on drop.
    /// The guard restores original terminal settings when dropped, even though
    /// we don't access the field directly after construction.
    ///
    /// The leading underscore suppresses the "unused field" warning while making
    /// it explicit this field exists solely for its `Drop` implementation.
    _term_mode: Option<crate::engine::terminal::AutoModeGuard>,
}

/// Events that can occur when reading input.
enum InputEvent {
    /// A regular character was read.
    Char(char),
    /// End of file was reached.
    Eof,
    /// The read operation timed out.
    Timeout,
    /// Ctrl+C was pressed.
    CtrlC,
    /// Ctrl+D was pressed.
    CtrlD,
}

enum ByteEvent {
    Byte(u8),
    Eof,
    Timeout,
}

impl InputReader {
    /// Creates a new input reader with optional timeout.
    fn new(
        input: crate::engine::openfiles::OpenFile,
        timeout: Option<Duration>,
        term_mode: Option<crate::engine::terminal::AutoModeGuard>,
    ) -> Self {
        Self {
            input,
            deadline: timeout.map(|t| Instant::now() + t),
            buffer: [0; 4],
            _term_mode: term_mode,
        }
    }

    /// Checks if input is immediately available (for `-t 0`). Returns `false` if an error
    /// occurs while checking for available input.
    fn check_input_available(&self) -> bool {
        crate::engine::sys::poll::poll_for_input(&self.input, Duration::ZERO).unwrap_or(false)
    }

    fn read_byte_event(&mut self) -> Result<ByteEvent, crate::engine::Error> {
        if let Some(deadline) = self.deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(ByteEvent::Timeout);
            }

            match crate::engine::sys::poll::poll_for_input(&self.input, remaining) {
                Ok(true) => {}
                Ok(false) => return Ok(ByteEvent::Timeout),
                Err(e) => return Err(e.into()),
            }
        }

        let mut byte = [0];
        let n = self.input.read(&mut byte)?;
        Ok(if n == 0 {
            ByteEvent::Eof
        } else {
            ByteEvent::Byte(byte[0])
        })
    }

    /// Reads the next input event, handling timeout and control characters.
    fn read_event(&mut self) -> Result<InputEvent, crate::engine::Error> {
        let first = match self.read_byte_event()? {
            ByteEvent::Byte(byte) => byte,
            ByteEvent::Eof => return Ok(InputEvent::Eof),
            ByteEvent::Timeout => return Ok(InputEvent::Timeout),
        };

        if first.is_ascii() {
            let ch = char::from(first);
            return Ok(match ch {
                CTRL_C => InputEvent::CtrlC,
                CTRL_D => InputEvent::CtrlD,
                _ => InputEvent::Char(ch),
            });
        }

        let width = utf8_char_width(first).ok_or_else(invalid_utf8_input)?;
        self.buffer[0] = first;
        for index in 1..width {
            self.buffer[index] = match self.read_byte_event()? {
                ByteEvent::Byte(byte) => byte,
                ByteEvent::Eof => return Err(invalid_utf8_input().into()),
                ByteEvent::Timeout => return Ok(InputEvent::Timeout),
            };
        }

        Ok(InputEvent::Char(decode_utf8_char(&self.buffer[..width])?))
    }
}

fn utf8_char_width(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn decode_utf8_char(bytes: &[u8]) -> Result<char, crate::engine::Error> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|value| value.chars().next())
        .ok_or_else(|| invalid_utf8_input().into())
}

fn invalid_utf8_input() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "read input is not valid UTF-8",
    )
}

/// Configuration for line reading behavior.
struct LineReaderConfig {
    /// Character that terminates input (None for -N mode).
    delimiter: Option<char>,
    /// Maximum characters to read (for -n or -N).
    char_limit: Option<usize>,
    /// Whether to process backslash escapes (false for -r mode).
    process_escapes: bool,
}

/// Reads a complete line of input using the given reader and configuration.
///
/// Returns a `ReadResult` indicating success, EOF, timeout, or interruption.
///
/// Note on character counting for `-n` limit:
/// Bash counts OUTPUT characters (after escape processing) toward the limit.
/// For example, with `-n 3` and input `a\bc` (4 bytes):
/// - Bash processes: 'a' (output 1), '\b' → 'b' (output 2), 'c' (output 3) → "abc"
/// - The backslash is consumed but doesn't count toward the limit
fn read_line_with_reader(
    reader: &mut InputReader,
    config: &LineReaderConfig,
    stderr: &mut impl Write,
    manual_echo: bool,
) -> Result<ReadResult, crate::engine::Error> {
    let mut line = String::new();
    let mut output_char_count = 0;
    let mut pending_backslash = false;

    if config.char_limit == Some(0) {
        return Ok(ReadResult::Line(line));
    }

    loop {
        let event = reader.read_event()?;

        match event {
            InputEvent::Eof => {
                // Bash discards pending backslash on EOF.
                return Ok(ReadResult::Eof(if line.is_empty() {
                    None
                } else {
                    Some(line)
                }));
            }

            InputEvent::Timeout => {
                // Include pending backslash on timeout (different from EOF).
                if pending_backslash {
                    line.push(BACKSLASH);
                }
                return Ok(ReadResult::TimedOut(if line.is_empty() {
                    None
                } else {
                    Some(line)
                }));
            }

            InputEvent::CtrlC => {
                return Ok(ReadResult::Interrupted);
            }

            InputEvent::CtrlD => {
                // At line start = EOF, mid-input = flush current input.
                // Bash discards pending backslash here too.
                return Ok(if line.is_empty() && !pending_backslash {
                    ReadResult::Eof(None)
                } else {
                    ReadResult::Line(line)
                });
            }

            InputEvent::Char(ch) => {
                if manual_echo {
                    write!(stderr, "{ch}")?;
                    stderr.flush()?;
                }

                // Handle backslash escape processing (when enabled).
                if config.process_escapes {
                    if pending_backslash {
                        pending_backslash = false;

                        // Backslash-delimiter is line continuation.
                        if let Some(delim) = config.delimiter
                            && ch == delim
                        {
                            continue; // Line continuation.
                        }

                        // For other chars, add char literally (backslash consumed).
                        if push_output_char(
                            &mut line,
                            &mut output_char_count,
                            ch,
                            config.char_limit,
                        ) {
                            return Ok(ReadResult::Line(line));
                        }
                        continue;
                    }

                    if ch == BACKSLASH {
                        pending_backslash = true;
                        continue;
                    }
                }

                // Check for delimiter.
                if let Some(delim) = config.delimiter
                    && ch == delim
                {
                    return Ok(ReadResult::Line(line));
                }

                // Ignore non-whitespace control characters.
                if ch.is_ascii_control() && !ch.is_ascii_whitespace() {
                    continue;
                }

                if push_output_char(&mut line, &mut output_char_count, ch, config.char_limit) {
                    return Ok(ReadResult::Line(line));
                }
            }
        }
    }
}

fn push_output_char(
    line: &mut String,
    output_char_count: &mut usize,
    ch: char,
    char_limit: Option<usize>,
) -> bool {
    line.push(ch);
    *output_char_count += 1;
    char_limit.is_some_and(|limit| *output_char_count >= limit)
}

struct ReadWorker {
    delimiter: Option<String>,
    prompt: Option<String>,
    return_after_n_chars: Option<usize>,
    return_after_n_chars_no_delimiter: Option<usize>,
    raw_mode: bool,
    silent: bool,
}

impl From<&ReadCommand> for ReadWorker {
    fn from(command: &ReadCommand) -> Self {
        Self {
            delimiter: command.delimiter.clone(),
            prompt: command.prompt.clone(),
            return_after_n_chars: command.return_after_n_chars,
            return_after_n_chars_no_delimiter: command.return_after_n_chars_no_delimiter,
            raw_mode: command.raw_mode,
            silent: command.silent,
        }
    }
}

impl ReadWorker {
    /// Reads a line of input, optionally with a timeout.
    ///
    /// Handles backslash escape processing:
    /// - Without `-r`: backslash-newline is line continuation, other backslashes escape the next
    ///   char
    /// - With `-r`: backslash is treated as a literal character
    fn read_line(
        &self,
        input_file: crate::engine::openfiles::OpenFile,
        mut stderr_file: impl std::io::Write,
        timeout: Option<Duration>,
    ) -> Result<ReadResult, crate::engine::Error> {
        let manual_echo = should_manually_echo(input_file.is_terminal(), self.silent);
        let term_mode = setup_terminal_settings(&input_file)?;

        // Display prompt on stderr, but only if input is from a terminal (per bash behavior).
        if let Some(prompt) = &self.prompt
            && input_file.is_terminal()
        {
            write!(stderr_file, "{prompt}")?;
            stderr_file.flush()?;
        }

        // Determine delimiter based on options.
        let delimiter = if self.return_after_n_chars_no_delimiter.is_some() {
            None
        } else if let Some(delimiter_str) = &self.delimiter {
            if delimiter_str.is_empty() {
                Some(NUL_DELIMITER)
            } else {
                delimiter_str.chars().next()
            }
        } else {
            Some(DEFAULT_DELIMITER)
        };

        let char_limit = self
            .return_after_n_chars_no_delimiter
            .or(self.return_after_n_chars);

        // Create the input reader.
        let mut reader = InputReader::new(input_file, timeout, term_mode);

        // Handle -t 0 special case: just check if input is available without reading.
        if timeout == Some(Duration::ZERO) {
            return Ok(if reader.check_input_available() {
                ReadResult::InputReady
            } else {
                ReadResult::InputNotReady
            });
        }

        // Configure and perform the read.
        let config = LineReaderConfig {
            delimiter,
            char_limit,
            process_escapes: !self.raw_mode,
        };

        read_line_with_reader(&mut reader, &config, &mut stderr_file, manual_echo)
    }
}

fn setup_terminal_settings(
    file: &crate::engine::openfiles::OpenFile,
) -> Result<Option<crate::engine::terminal::AutoModeGuard>, crate::engine::Error> {
    let mode = crate::engine::terminal::AutoModeGuard::new(file.try_clone()?).ok();
    if let Some(mode) = &mode {
        mode.apply_settings(&crate::engine::terminal::Settings::character_input())?;
    }

    Ok(mode)
}

impl ReadCommand {
    /// Validates the timeout value and returns an error result if invalid.
    ///
    /// Returns `Ok(Some(result))` if the timeout is invalid (caller should return early),
    /// `Ok(None)` if the timeout is valid or not specified.
    ///
    /// TODO(read): Bash uses $TMOUT as a default timeout for `read` when -t is not specified.
    fn validate_timeout(
        &self,
        context: &crate::engine::ExecutionContext<'_>,
    ) -> Result<Option<crate::engine::ExecutionResult>, crate::engine::Error> {
        if let Some(timeout) = self.timeout_in_seconds
            && timeout < 0.0
        {
            writeln!(
                context.stderr(),
                "{}: -t: invalid timeout specification",
                context.command_name
            )?;
            return Ok(Some(crate::engine::ExecutionResult::general_error()));
        }
        Ok(None)
    }
}

fn should_manually_echo(input_is_terminal: bool, silent: bool) -> bool {
    input_is_terminal && !silent
}

/// Splits a line by IFS (Internal Field Separator) according to shell rules.
///
/// Shell IFS splitting has special rules:
/// - Whitespace IFS chars (space, tab, newline) are "IFS whitespace"
/// - Leading/trailing IFS whitespace is trimmed from the input
/// - Consecutive IFS whitespace chars act as a single delimiter
/// - Non-whitespace IFS chars each act as individual delimiters
/// - Trailing non-whitespace delimiter does NOT create an empty final field
///
/// # Arguments
/// * `ifs` - The IFS string (typically " \t\n")
/// * `line` - The input line to split
/// * `max_fields` - Optional limit on number of fields (for `read var1 var2`)
fn split_line_by_ifs(ifs: &str, line: &str, max_fields: Option<usize>) -> VecDeque<String> {
    let ifs_chars: Vec<char> = ifs.chars().collect();

    // Helper to check if a char is IFS whitespace (space, tab, or newline AND in IFS).
    let is_ifs_whitespace =
        |c: char| -> bool { (c == ' ' || c == '\t' || c == '\n') && ifs_chars.contains(&c) };

    // Trim leading/trailing IFS whitespace from the input.
    let trimmed_line = line.trim_matches(&is_ifs_whitespace);
    if trimmed_line.is_empty() {
        return VecDeque::new();
    }

    let max_fields = max_fields.unwrap_or(usize::MAX);

    // State machine for splitting:
    // - `consuming_whitespace_run`: Currently skipping consecutive IFS whitespace
    // - `prev_was_non_ws_delim`: Previous char was a non-whitespace delimiter
    // - `collecting_remainder`: We've hit max_fields, collect everything into last field
    let mut fields = VecDeque::new();
    let mut current_field = String::new();
    let mut consuming_whitespace_run = false;
    let mut prev_was_non_ws_delim = false;
    let mut collecting_remainder = false;

    for c in trimmed_line.chars() {
        // Skip consecutive IFS whitespace (they act as single delimiter).
        if consuming_whitespace_run && is_ifs_whitespace(c) {
            continue;
        }
        consuming_whitespace_run = false;

        let is_delimiter = ifs_chars.contains(&c);
        let at_field_limit = fields.len() + 1 >= max_fields;

        if !at_field_limit && is_delimiter {
            // Normal case: delimiter ends current field, start new one.
            fields.push_back(std::mem::take(&mut current_field));
            consuming_whitespace_run = is_ifs_whitespace(c);
            prev_was_non_ws_delim = !consuming_whitespace_run;
        } else if at_field_limit && !collecting_remainder && is_delimiter {
            // At field limit but haven't started last field content yet.
            // Skip leading IFS whitespace for the final field.
            if is_ifs_whitespace(c) {
                consuming_whitespace_run = true;
            } else {
                // Non-whitespace delimiters at boundary: include in remainder.
                // e.g., "x::y" with IFS=":" and 2 vars gives ["x", ":y"]
                collecting_remainder = true;
                current_field.push(c);
            }
        } else {
            // Regular character: add to current field.
            collecting_remainder = at_field_limit;
            current_field.push(c);
            prev_was_non_ws_delim = false;
        }
    }

    // Finalize: push last field unless it's empty AND we ended with non-ws delimiter.
    // e.g., "a,b,c," with IFS="," gives ["a", "b", "c"], not ["a", "b", "c", ""].
    if !current_field.is_empty() || !prev_was_non_ws_delim {
        fields.push_back(current_field);
    }

    fields
}

#[cfg(test)]
mod tests {
    use itertools::assert_equal;

    use super::*;

    #[compio::test]
    async fn blocking_worker_reads_complete_utf8_line() -> anyhow::Result<()> {
        let (reader, mut writer) = std::io::pipe()?;
        let writer_task =
            compio::runtime::spawn_blocking(move || writer.write_all("界a\n".as_bytes()));
        let worker = ReadWorker {
            delimiter: None,
            prompt: None,
            return_after_n_chars: None,
            return_after_n_chars_no_delimiter: None,
            raw_mode: false,
            silent: false,
        };

        let result = compio::runtime::spawn_blocking(move || {
            worker.read_line(reader.into(), Vec::<u8>::new(), None)
        })
        .await
        .map_err(|err| anyhow::anyhow!("read worker panicked: {err:?}"))??;
        writer_task
            .await
            .map_err(|err| anyhow::anyhow!("writer worker panicked: {err:?}"))??;

        match result {
            ReadResult::Line(line) => assert_eq!(line, "界a"),
            _ => anyhow::bail!("unexpected read result"),
        }
        Ok(())
    }

    #[test]
    fn rejects_non_finite_timeouts() {
        assert!(parse_timeout_option("NaN").is_err());
        assert!(parse_timeout_option("inf").is_err());
        assert!(parse_timeout_option("-inf").is_err());
        assert_eq!(parse_timeout_option("0.25"), Ok(0.25));
    }

    #[test]
    fn selects_manual_echo_only_for_non_silent_terminal_input() {
        assert!(should_manually_echo(true, false));
        assert!(!should_manually_echo(true, true));
        assert!(!should_manually_echo(false, false));
        assert!(!should_manually_echo(false, true));
    }

    #[test]
    fn decodes_utf8_and_counts_characters_for_n_options() {
        assert_eq!(decode_utf8_char("界".as_bytes()).unwrap(), '界');

        let mut line = String::new();
        let mut count = 0;
        assert!(!push_output_char(&mut line, &mut count, '界', Some(2)));
        assert!(push_output_char(&mut line, &mut count, 'a', Some(2)));
        assert_eq!(line, "界a");
        assert_eq!(count, 2);

        let command =
            <ReadCommand as builtins::Command>::new(["read", "-n", "2"].map(String::from)).unwrap();
        assert_eq!(command.return_after_n_chars, Some(2));

        let command =
            <ReadCommand as builtins::Command>::new(["read", "-N", "2"].map(String::from)).unwrap();
        assert_eq!(command.return_after_n_chars_no_delimiter, Some(2));
    }

    // ==================== split_line_by_ifs tests ====================

    #[test]
    fn test_split_line_by_ifs_basic() {
        let result = split_line_by_ifs(",", "a,b,c", None);
        assert_equal(result, VecDeque::from(vec!["a", "b", "c"]));
    }

    #[test]
    fn test_split_line_by_ifs_leading_or_trailing_space() {
        let result = split_line_by_ifs(" ", "  a b c ", None);
        assert_equal(result, VecDeque::from(vec!["a", "b", "c"]));
    }

    #[test]
    fn test_split_line_by_ifs_extra_interior_space() {
        let result = split_line_by_ifs(" ", "a  b c", None);
        assert_equal(result, VecDeque::from(vec!["a", "b", "c"]));
    }

    #[test]
    fn test_split_line_by_ifs_leading_non_space_delimiter() {
        let result = split_line_by_ifs(",", ",a,b,c", None);
        assert_equal(result, VecDeque::from(vec!["", "a", "b", "c"]));
    }

    #[test]
    fn test_split_line_by_ifs_trailing_non_space_delimiter() {
        // Bash does NOT include empty trailing field when input ends with non-ws delimiter.
        let result = split_line_by_ifs(",", "a,b,c,", None);
        assert_equal(result, VecDeque::from(vec!["a", "b", "c"]));
    }

    #[test]
    fn test_split_line_by_ifs_max_fields() {
        // With max_fields=2, remainder goes into second field.
        let result = split_line_by_ifs(" ", "a b c d", Some(2));
        assert_equal(result, VecDeque::from(vec!["a", "b c d"]));
    }

    #[test]
    fn test_split_line_by_ifs_max_fields_with_non_ws_delimiter() {
        // With max_fields and non-whitespace delimiter.
        let result = split_line_by_ifs(",", "a,b,c,d", Some(2));
        assert_equal(result, VecDeque::from(vec!["a", "b,c,d"]));
    }

    #[test]
    fn test_split_line_by_ifs_consecutive_delimiters_at_boundary() {
        // Consecutive non-whitespace delimiters at field boundary should be preserved.
        // e.g., "x::y" with IFS=":" and 2 vars gives ["x", ":y"]
        let result = split_line_by_ifs(":", "x::y", Some(2));
        assert_equal(result, VecDeque::from(vec!["x", ":y"]));

        // Triple delimiter at boundary.
        let result = split_line_by_ifs(":", "x:::y", Some(2));
        assert_equal(result, VecDeque::from(vec!["x", "::y"]));

        // Delimiter in middle of remainder is also preserved.
        let result = split_line_by_ifs(":", "x:y:z:w", Some(2));
        assert_equal(result, VecDeque::from(vec!["x", "y:z:w"]));
    }

    #[test]
    fn test_split_line_by_ifs_mixed_delimiters() {
        // Mixed whitespace and non-whitespace in IFS.
        let result = split_line_by_ifs(": ", "a:b  c:d", None);
        assert_equal(result, VecDeque::from(vec!["a", "b", "c", "d"]));
    }

    #[test]
    fn test_split_line_by_ifs_empty_input() {
        let result = split_line_by_ifs(" ", "", None);
        assert_equal(result, VecDeque::<String>::new());
    }

    #[test]
    fn test_split_line_by_ifs_whitespace_only() {
        let result = split_line_by_ifs(" ", "   ", None);
        assert_equal(result, VecDeque::<String>::new());
    }

    #[test]
    fn test_split_line_by_ifs_consecutive_non_ws_delimiters() {
        // Consecutive non-whitespace delimiters create empty fields.
        let result = split_line_by_ifs(",", "a,,b", None);
        assert_equal(result, VecDeque::from(vec!["a", "", "b"]));
    }

    // ==================== build_array_fields tests ====================

    #[test]
    fn test_build_array_fields_basic() {
        let result = build_array_fields(Some("a b c"), " ", false);
        assert_eq!(
            result,
            vec![
                (None, "a".to_string()),
                (None, "b".to_string()),
                (None, "c".to_string())
            ]
        );
    }

    #[test]
    fn test_build_array_fields_skip_splitting() {
        // With -N option, entire input goes as single element.
        let result = build_array_fields(Some("a b c"), " ", true);
        assert_eq!(result, vec![(None, "a b c".to_string())]);
    }

    #[test]
    fn test_build_array_fields_none_input() {
        let result = build_array_fields(None, " ", false);
        assert!(result.is_empty());
    }

    // ==================== build_variable_fields tests ====================

    #[test]
    fn test_build_variable_fields_basic() {
        let result = build_variable_fields(Some("a b c"), " ", false, 3);
        assert_equal(result, VecDeque::from(vec!["a", "b", "c"]));
    }

    #[test]
    fn test_build_variable_fields_fewer_vars_than_fields() {
        // Last variable gets remainder.
        let result = build_variable_fields(Some("a b c d"), " ", false, 2);
        assert_equal(result, VecDeque::from(vec!["a", "b c d"]));
    }

    #[test]
    fn test_build_variable_fields_skip_splitting() {
        // With -N option, entire input goes to first variable.
        let result = build_variable_fields(Some("a b c"), " ", true, 3);
        assert_equal(result, VecDeque::from(vec!["a b c"]));
    }

    #[test]
    fn test_build_variable_fields_none_input() {
        let result = build_variable_fields(None, " ", false, 3);
        assert!(result.is_empty());
    }
}
