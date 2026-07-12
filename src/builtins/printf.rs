use std::io::Write;

use crate::engine::{ErrorKind, ExecutionResult, builtins, escape, expansion};

// 限制单个格式字段, 避免用户输入导致无界输出或分配.
const MAX_FORMAT_FIELD_SIZE: usize = 1024 * 1024;
const WRITE_CHUNK_SIZE: usize = 8 * 1024;

/// Format a string.
pub(crate) struct PrintfCommand {
    /// If specified, the output of the command is assigned to this variable.
    output_variable: Option<String>,

    /// Format string + arguments to the format string.
    format_and_args: Vec<String>,
}

impl builtins::Command for PrintfCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let mut output_variable = None;
        let mut format_and_args = Vec::new();

        if let Some(arg) = args.next_arg() {
            match arg.as_str() {
                "--" => format_and_args.extend(args.rest()),
                "-v" => {
                    output_variable = Some(args.next_value("-v")?);
                    if args.peek() == Some("--") {
                        let _ = args.next_arg();
                    }
                    format_and_args.extend(args.rest());
                }
                _ if arg.starts_with("-v") && arg.len() > 2 => {
                    output_variable = Some(arg[2..].to_owned());
                    if args.peek() == Some("--") {
                        let _ = args.next_arg();
                    }
                    format_and_args.extend(args.rest());
                }
                _ => {
                    format_and_args.push(arg);
                    format_and_args.extend(args.rest());
                }
            }
        }

        if format_and_args.is_empty() {
            return Err(String::from("missing operand"));
        }

        Ok(Self {
            output_variable,
            format_and_args,
        })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
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

#[derive(Clone, Copy, Default)]
struct FormatSpecifier {
    conversion: char,
    left_justify: bool,
    force_sign: bool,
    space_sign: bool,
    alternate: bool,
    zero_pad: bool,
    width: Option<usize>,
    precision: Option<usize>,
}

fn read_format_specifier(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<FormatSpecifier, crate::engine::Error> {
    let mut spec = FormatSpecifier::default();

    while let Some(flag) = chars.peek().copied() {
        match flag {
            '-' => spec.left_justify = true,
            '+' => spec.force_sign = true,
            ' ' => spec.space_sign = true,
            '#' => spec.alternate = true,
            '0' => spec.zero_pad = true,
            _ => break,
        }
        let _ = chars.next();
    }

    spec.width = parse_format_number(chars, "width")?;
    if chars.next_if_eq(&'.').is_some() {
        spec.precision = Some(parse_format_number(chars, "precision")?.unwrap_or(0));
    }

    let Some(conversion) = chars.next() else {
        return Err(ErrorKind::PrintfInvalidUsage("missing format specifier".into()).into());
    };
    if !conversion.is_ascii_alphabetic() {
        return Err(ErrorKind::PrintfInvalidUsage(format!(
            "unsupported format modifier: {conversion}"
        ))
        .into());
    }
    spec.conversion = conversion;
    Ok(spec)
}

fn parse_format_number(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    field_name: &str,
) -> Result<Option<usize>, crate::engine::Error> {
    let mut value = None::<usize>;
    while let Some(digit) = chars.peek().and_then(|ch| ch.to_digit(10)) {
        let _ = chars.next();
        let next_value = value
            .unwrap_or(0)
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit as usize))
            .ok_or_else(|| {
                ErrorKind::PrintfInvalidUsage(format!("format {field_name} is too large"))
            })?;
        if next_value > MAX_FORMAT_FIELD_SIZE {
            return Err(ErrorKind::PrintfInvalidUsage(format!(
                "format {field_name} exceeds the maximum of {MAX_FORMAT_FIELD_SIZE}"
            ))
            .into());
        }
        value = Some(next_value);
    }
    Ok(value)
}

fn write_formatted_arg(
    mut writer: impl Write,
    spec: FormatSpecifier,
    arg: &str,
) -> Result<(), crate::engine::Error> {
    match spec.conversion {
        's' => write_padded_text(
            &mut writer,
            truncate_to_precision(arg, spec.precision),
            spec,
        )?,
        'q' => {
            let value = truncate_to_precision(arg, spec.precision);
            let escaped = escape::quote_if_needed(&value, escape::QuoteMode::BackslashEscape);
            write_padded_text(&mut writer, escaped.as_ref().to_owned(), spec)?;
        }
        'c' => write_padded_text(
            &mut writer,
            arg.chars().next().unwrap_or('\0').to_string(),
            spec,
        )?,
        'd' | 'i' => {
            let value = parse_i64_arg(arg)?;
            let sign = if value.is_negative() {
                Some('-')
            } else if spec.force_sign {
                Some('+')
            } else if spec.space_sign {
                Some(' ')
            } else {
                None
            };
            write_number(&mut writer, value.unsigned_abs(), 10, sign, spec)?;
        }
        'u' => write_number(&mut writer, parse_u64_arg(arg)?, 10, None, spec)?,
        'o' => write_number(&mut writer, parse_u64_arg(arg)?, 8, None, spec)?,
        'x' | 'X' => write_number(&mut writer, parse_u64_arg(arg)?, 16, None, spec)?,
        other => {
            return Err(ErrorKind::PrintfInvalidUsage(format!(
                "unsupported format specifier: %{other}"
            ))
            .into());
        }
    }

    Ok(())
}

fn truncate_to_precision(value: &str, precision: Option<usize>) -> String {
    precision.map_or_else(
        || value.to_owned(),
        |precision| value.chars().take(precision).collect(),
    )
}

fn write_padded_text(
    mut writer: impl Write,
    value: String,
    spec: FormatSpecifier,
) -> Result<(), crate::engine::Error> {
    let padding = spec
        .width
        .unwrap_or(0)
        .saturating_sub(value.chars().count());
    if !spec.left_justify {
        write_repeated_byte(&mut writer, b' ', padding)?;
    }
    write!(writer, "{value}")?;
    if spec.left_justify {
        write_repeated_byte(&mut writer, b' ', padding)?;
    }
    Ok(())
}

fn write_repeated_byte(
    mut writer: impl Write,
    byte: u8,
    mut count: usize,
) -> Result<(), std::io::Error> {
    let chunk = [byte; WRITE_CHUNK_SIZE];
    while count > 0 {
        let chunk_len = count.min(chunk.len());
        writer.write_all(&chunk[..chunk_len])?;
        count -= chunk_len;
    }
    Ok(())
}

fn write_number(
    mut writer: impl Write,
    value: u64,
    radix: u32,
    sign: Option<char>,
    spec: FormatSpecifier,
) -> Result<(), crate::engine::Error> {
    let mut digits = match (radix, spec.conversion) {
        (8, _) => format!("{value:o}"),
        (16, 'X') => format!("{value:X}"),
        (16, _) => format!("{value:x}"),
        _ => value.to_string(),
    };

    if spec.precision == Some(0) && value == 0 {
        digits.clear();
    }
    let precision_zeroes = spec.precision.unwrap_or(0).saturating_sub(digits.len());

    let prefix = if spec.alternate {
        match spec.conversion {
            'o' if precision_zeroes == 0 && !digits.starts_with('0') => "0",
            'x' if value != 0 => "0x",
            'X' if value != 0 => "0X",
            _ => "",
        }
    } else {
        ""
    };
    let content_len = usize::from(sign.is_some()) + prefix.len() + precision_zeroes + digits.len();
    let padding = spec.width.unwrap_or(0).saturating_sub(content_len);

    if !(spec.left_justify || spec.zero_pad && spec.precision.is_none()) {
        write_repeated_byte(&mut writer, b' ', padding)?;
    }
    if let Some(sign) = sign {
        write!(writer, "{sign}")?;
    }
    write!(writer, "{prefix}")?;
    if !spec.left_justify && spec.zero_pad && spec.precision.is_none() {
        write_repeated_byte(&mut writer, b'0', padding)?;
    }
    write_repeated_byte(&mut writer, b'0', precision_zeroes)?;
    write!(writer, "{digits}")?;
    if spec.left_justify {
        write_repeated_byte(&mut writer, b' ', padding)?;
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

    #[test]
    fn test_sprintf_honors_format_modifiers() -> Result<()> {
        assert_eq!(sprintf("%05d", &["12"])?, "00012");
        assert_eq!(sprintf("%-05d", &["12"])?, "12   ");
        assert_eq!(sprintf("%05.3d", &["12"])?, "  012");
        assert_eq!(sprintf("%-5s", &["xy"])?, "xy   ");
        assert_eq!(sprintf("%05s", &["xy"])?, "   xy");
        assert_eq!(sprintf("%.2s", &["世界a"])?, "世界");
        assert_eq!(sprintf("%#05x", &["16"])?, "0x010");
        assert_eq!(sprintf("%#5.0x", &["0"])?, "     ");
        assert_eq!(sprintf("% d", &["3"])?, " 3");
        assert_eq!(sprintf("% +d", &["3"])?, "+3");
        assert_eq!(sprintf("%+d", &["3"])?, "+3");
        Ok(())
    }

    #[test]
    fn test_sprintf_honors_alternate_octal_form() -> Result<()> {
        assert_eq!(sprintf("%#o", &["0"])?, "0");
        assert_eq!(sprintf("%#.0o", &["0"])?, "0");
        assert_eq!(sprintf("%#5.0o", &["0"])?, "    0");
        assert_eq!(sprintf("%#.3o", &["8"])?, "010");
        assert_eq!(sprintf("%#05o", &["8"])?, "00010");
        Ok(())
    }

    #[test]
    fn test_sprintf_rejects_excessive_width_and_precision() {
        assert!(sprintf("%1048577s", &["x"]).is_err());
        assert!(sprintf("%.1048577d", &["1"]).is_err());
        assert!(sprintf("%999999999999999999999999s", &["x"]).is_err());
        assert!(sprintf("%.999999999999999999999999d", &["1"]).is_err());
    }

    #[test]
    fn test_sprintf_rejects_unsupported_modifier() {
        assert!(sprintf("%*s", &["5", "x"]).is_err());
    }
}
