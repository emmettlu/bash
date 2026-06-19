use clap::Parser;
use std::io::Write;

use crate::engine::{ErrorKind, ExecutionResult, builtins, escape, expansion};

/// Format a string.
#[derive(Parser)]
#[clap(disable_help_flag = true, disable_version_flag = true)]
pub(crate) struct PrintfCommand {
    /// If specified, the output of the command is assigned to this variable.
    #[arg(short = 'v')]
    output_variable: Option<String>,

    /// Format string + arguments to the format string.
    #[arg(trailing_var_arg = true, required = true, allow_hyphen_values = true)]
    format_and_args: Vec<String>,
}

impl builtins::Command for PrintfCommand {
    type Error = crate::engine::Error;

    async fn execute<SE: crate::engine::ShellExtensions>(
        &self,
        context: crate::engine::ExecutionContext<'_, SE>,
    ) -> Result<ExecutionResult, Self::Error> {
        if let Some(variable_name) = &self.output_variable {
            let mut result: Vec<u8> = vec![];
            format(self.format_and_args.as_slice(), &mut result)?;

            let result_str = String::from_utf8(result).map_err(|_| {
                crate::engine::ErrorKind::PrintfInvalidUsage("invalid UTF-8 output".into())
            })?;

            expansion::assign_to_named_parameter(
                context.shell,
                &context.params,
                variable_name,
                result_str,
            )
            .await?;
        } else {
            format(self.format_and_args.as_slice(), context.stdout())?;
            context.stdout().flush()?;
        }

        Ok(ExecutionResult::success())
    }
}

fn format(format_and_args: &[String], writer: impl Write) -> Result<(), crate::engine::Error> {
    match format_and_args {
        [fmt, arg] if fmt == "%q" => format_special_case_for_percent_q(None, arg, writer),
        [fmt, arg] if fmt == "~%q" => format_special_case_for_percent_q(Some("~"), arg, writer),
        [fmt, args @ ..] => format_simple(fmt, args, writer),
        [] => Err(ErrorKind::PrintfInvalidUsage("missing operand".into()).into()),
    }
}

fn format_special_case_for_percent_q(
    prefix: Option<&str>,
    arg: &str,
    mut writer: impl Write,
) -> Result<(), crate::engine::Error> {
    let mut result = escape::quote_if_needed(arg, escape::QuoteMode::BackslashEscape).to_string();

    if let Some(prefix) = prefix {
        result.insert_str(0, prefix);
    }

    write!(writer, "{result}")?;

    Ok(())
}

fn format_simple(
    format_string: &str,
    args: &[String],
    mut writer: impl Write,
) -> Result<(), crate::engine::Error> {
    let expanded_format = expand_format_escapes(format_string)?;
    let mut arg_index = 0usize;
    let mut chars = expanded_format.chars().peekable();

    loop {
        let mut emitted_specifier = false;

        while let Some(ch) = chars.next() {
            if ch != '%' {
                write!(writer, "{ch}")?;
                continue;
            }

            if chars.next_if_eq(&'%').is_some() {
                writer.write_all(b"%")?;
                continue;
            }

            let spec = read_format_specifier(&mut chars)?;
            let arg = args.get(arg_index).map_or("", String::as_str);
            arg_index += 1;
            emitted_specifier = true;
            write_formatted_arg(&mut writer, spec, arg)?;
        }

        if args.is_empty() || arg_index >= args.len() || !emitted_specifier {
            break;
        }

        chars = expanded_format.chars().peekable();
    }

    Ok(())
}

fn expand_format_escapes(format_string: &str) -> Result<String, crate::engine::Error> {
    let (bytes, _) =
        escape::expand_backslash_escapes(format_string, escape::EscapeExpansionMode::EchoBuiltin)?;
    String::from_utf8(bytes)
        .map_err(|_| ErrorKind::PrintfInvalidUsage("invalid UTF-8 format string".into()).into())
}

fn read_format_specifier(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<char, crate::engine::Error> {
    for ch in chars.by_ref() {
        if ch.is_ascii_alphabetic() || ch == '%' {
            return Ok(ch);
        }
    }

    Err(ErrorKind::PrintfInvalidUsage("missing format specifier".into()).into())
}

fn write_formatted_arg(
    mut writer: impl Write,
    spec: char,
    arg: &str,
) -> Result<(), crate::engine::Error> {
    match spec {
        's' => write!(writer, "{arg}")?,
        'q' => write!(
            writer,
            "{}",
            escape::quote_if_needed(arg, escape::QuoteMode::BackslashEscape)
        )?,
        'c' => write!(writer, "{}", arg.chars().next().unwrap_or('\0'))?,
        'd' | 'i' => write!(writer, "{}", parse_i64_arg(arg)?)?,
        'u' => write!(writer, "{}", parse_u64_arg(arg)?)?,
        'o' => write!(writer, "{:o}", parse_u64_arg(arg)?)?,
        'x' => write!(writer, "{:x}", parse_u64_arg(arg)?)?,
        'X' => write!(writer, "{:X}", parse_u64_arg(arg)?)?,
        other => {
            return Err(ErrorKind::PrintfInvalidUsage(format!(
                "unsupported format specifier: %{other}"
            ))
            .into());
        }
    }

    Ok(())
}

fn parse_i64_arg(arg: &str) -> Result<i64, crate::engine::Error> {
    if arg.is_empty() {
        return Ok(0);
    }

    arg.parse::<i64>().map_err(|err| {
        ErrorKind::PrintfInvalidUsage(format!("invalid integer argument '{arg}': {err}")).into()
    })
}

fn parse_u64_arg(arg: &str) -> Result<u64, crate::engine::Error> {
    if arg.is_empty() {
        return Ok(0);
    }

    arg.parse::<u64>().map_err(|err| {
        ErrorKind::PrintfInvalidUsage(format!("invalid integer argument '{arg}': {err}")).into()
    })
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn sprintf(format_string: &str, args: &[&str]) -> Result<String> {
        let mut result = vec![];
        let args = args
            .iter()
            .map(|arg| (*arg).to_string())
            .collect::<Vec<_>>();
        format_simple(format_string, &args, &mut result)?;

        Ok(String::from_utf8(result)?)
    }

    #[test]
    fn test_basic_sprintf() -> Result<()> {
        assert_eq!(sprintf("%s", &["xyz"])?, "xyz");
        assert_eq!(sprintf(r"%d\n", &["1"])?, "1\n");

        Ok(())
    }

    #[test]
    fn test_sprintf_without_args() -> Result<()> {
        assert_eq!(sprintf("xyz", &[])?, "xyz");
        assert_eq!(sprintf("%s|", &[])?, "|");

        Ok(())
    }

    #[test]
    fn test_sprintf_with_cycles() -> Result<()> {
        assert_eq!(sprintf("%s|", &["x", "y"])?, "x|y|");

        Ok(())
    }
}
