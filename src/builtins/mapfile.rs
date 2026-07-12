use std::io::{Read, Write};

use crate::engine::{ErrorKind, ExecutionResult, builtins, env, error, variables};

/// Read lines from standard input into an indexed array variable.
pub(crate) struct MapFileCommand {
    /// Delimiter to use (defaults to newline).
    delimiter: Option<String>,

    /// Maximum number of entries to read (0 means no limit).
    max_count: i64,

    /// Index into array at which to start assignment.
    origin: Option<i64>,

    /// Number of initial entries to skip.
    skip_count: i64,

    /// Whether or not to remove the delimiter from each read line.
    remove_delimiter: bool,

    /// File descriptor to read from (defaults to stdin).
    fd: crate::engine::ShellFd,

    /// Name of function to call for each group of lines.
    callback: Option<String>,

    /// Number of lines to pass the callback for each group.
    callback_group_size: i64,

    /// Name of array to read into.
    array_var_name: String,
}

impl builtins::Command for MapFileCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            delimiter: None,
            max_count: 0,
            origin: None,
            skip_count: 0,
            remove_delimiter: false,
            fd: 0,
            callback: None,
            callback_group_size: 5000,
            array_var_name: String::from("MAPFILE"),
        };
        let mut args = builtins::BuiltinArgs::new(args);
        let array_names = args.parse_short_options(|args, flag, flags, rest_start| match flag {
            't' => {
                command.remove_delimiter = true;
                Ok(builtins::ShortOptionDisposition::Continue)
            }
            'd' => {
                command.delimiter = Some(args.option_value(flags, rest_start, "-d")?);
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            'n' => {
                let value = args.option_value(flags, rest_start, "-n")?;
                command.max_count = parse_i64_option("-n", &value)?;
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            'O' => {
                let value = args.option_value(flags, rest_start, "-O")?;
                command.origin = Some(parse_i64_option("-O", &value)?);
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            's' => {
                let value = args.option_value(flags, rest_start, "-s")?;
                let skip_count = parse_i64_option("-s", &value)?;
                if skip_count < 0 {
                    return Err(format!("-s: invalid count: {value}"));
                }
                command.skip_count = skip_count;
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            'u' => {
                let value = args.option_value(flags, rest_start, "-u")?;
                command.fd = value
                    .parse()
                    .map_err(|_| format!("-u: invalid file descriptor: {value}"))?;
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            'C' => {
                command.callback = Some(args.option_value(flags, rest_start, "-C")?);
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            'c' => {
                let value = args.option_value(flags, rest_start, "-c")?;
                let callback_group_size = parse_i64_option("-c", &value)?;
                if callback_group_size < 1 {
                    return Err(format!("-c: invalid count: {value}"));
                }
                command.callback_group_size = callback_group_size;
                Ok(builtins::ShortOptionDisposition::StopParsingArgument)
            }
            _ => Err(format!("-{flag}: invalid option")),
        })?;

        match array_names.as_slice() {
            [] => {}
            [array_var_name] => command.array_var_name = array_var_name.clone(),
            _ => return Err(String::from("too many arguments")),
        }

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.callback_group_size != 5000 || self.callback.is_some() {
            return error::unimp("mapfile -C/-c is not yet implemented");
        }

        if let Some(origin) = self.origin
            && origin < 0
        {
            writeln!(
                context.stderr(),
                "{}: {origin}: invalid array origin",
                context.command_name
            )?;
            return Ok(ExecutionResult::general_error());
        }

        if let Some((_, var)) = context.shell.env().get(&self.array_var_name)
            && matches!(
                var.value(),
                variables::ShellValue::AssociativeArray(_)
                    | variables::ShellValue::Unset(
                        variables::ShellValueUnsetType::AssociativeArray
                    )
            )
        {
            writeln!(
                context.stderr(),
                "{}: {}: not an indexed array",
                context.command_name,
                self.array_var_name
            )?;
            return Ok(ExecutionResult::general_error());
        }

        let input_file = context
            .try_clone_fd(self.fd)?
            .ok_or_else(|| ErrorKind::BadFileDescriptor(self.fd))?;

        // worker 持有输入 fd 并完成整个阻塞读取循环, 数组赋值仍在 async 线程执行。
        let reader = MapFileReader::from(self);
        let results = compio::runtime::spawn_blocking(move || reader.read_entries(input_file))
            .await
            .map_err(|err| ErrorKind::ThreadingError(err.to_string()))??;

        if let Some(origin) = self.origin {
            // -O: preserve existing array, assign at offset.
            for (elem_idx, (_key, value)) in results.0.into_iter().enumerate() {
                // If the user is getting to wraparounds in *bash*, they got bigger problems.
                #[allow(clippy::cast_possible_wrap)]
                let elem_idx = elem_idx as i64;
                context.shell.env_mut().update_or_add_array_element(
                    &self.array_var_name,
                    (elem_idx + origin).to_string(),
                    value,
                    |_| Ok(()),
                    env::EnvironmentLookup::Anywhere,
                    env::EnvironmentScope::Global,
                )?;
            }
        } else {
            // No -O: replace the entire variable (clears existing).
            context.shell.env_mut().update_or_add(
                &self.array_var_name,
                variables::ShellValueLiteral::Array(results),
                |_| Ok(()),
                env::EnvironmentLookup::Anywhere,
                env::EnvironmentScope::Global,
            )?;
        }

        Ok(ExecutionResult::success())
    }
}

fn parse_i64_option(option: &str, value: &str) -> Result<i64, String> {
    value
        .parse()
        .map_err(|_| format!("{option}: invalid number: {value}"))
}

struct MapFileReader {
    delimiter: Option<String>,
    max_count: i64,
    skip_count: i64,
    remove_delimiter: bool,
}

impl From<&MapFileCommand> for MapFileReader {
    fn from(command: &MapFileCommand) -> Self {
        Self {
            delimiter: command.delimiter.clone(),
            max_count: command.max_count,
            skip_count: command.skip_count,
            remove_delimiter: command.remove_delimiter,
        }
    }
}

impl MapFileReader {
    fn read_entries(
        &self,
        mut input_file: crate::engine::openfiles::OpenFile,
    ) -> Result<variables::ArrayLiteral, crate::engine::Error> {
        let _term_mode = setup_terminal_settings(&input_file)?;

        let mut entries = vec![];
        let mut read_count = 0;
        let max_count = self.max_count.try_into()?;
        let delimiter = match &self.delimiter {
            Some(d) if d.is_empty() => b'\0',
            Some(d) => d.as_bytes().first().copied().unwrap_or(b'\n'),
            None => b'\n',
        };

        let mut buf = [0u8; 1];

        while max_count == 0 || entries.len() < max_count {
            let mut line = vec![];
            let mut saw_delimiter = false;

            loop {
                match input_file.read(&mut buf) {
                    Ok(0) => break,                                         // End of input
                    Ok(1) if buf[0] == b'\x03' => break,                    // Ctrl+C
                    Ok(1) if buf[0] == b'\x04' && line.is_empty() => break, // Ctrl+D
                    Ok(1) => {
                        let byte = buf[0];
                        line.push(byte);
                        if byte == delimiter {
                            saw_delimiter = true;
                            break;
                        }
                    }
                    Ok(_) => unreachable!("input can only be 0, 1, or error"),
                    Err(e) => return Err(e.into()),
                }
            }

            if line.is_empty() && !saw_delimiter {
                break;
            }

            if read_count < self.skip_count {
                read_count += 1;
                continue;
            }

            if self.remove_delimiter && line.ends_with(&[delimiter]) {
                line.pop();
            }

            let line_str = String::from_utf8_lossy(&line).to_string();

            entries.push((None, line_str));
        }

        Ok(variables::ArrayLiteral(entries))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[compio::test]
    async fn blocking_worker_returns_owned_entries() -> anyhow::Result<()> {
        let (reader, mut writer) = std::io::pipe()?;
        let writer_task =
            compio::runtime::spawn_blocking(move || writer.write_all(b"skip\nfirst\nsecond\n"));
        let worker = MapFileReader {
            delimiter: None,
            max_count: 2,
            skip_count: 1,
            remove_delimiter: true,
        };

        let entries = compio::runtime::spawn_blocking(move || worker.read_entries(reader.into()))
            .await
            .map_err(|err| anyhow::anyhow!("mapfile worker panicked: {err:?}"))??;
        writer_task
            .await
            .map_err(|err| anyhow::anyhow!("writer worker panicked: {err:?}"))??;

        assert_eq!(
            entries.0,
            vec![
                (None, String::from("first")),
                (None, String::from("second")),
            ]
        );
        Ok(())
    }
}
