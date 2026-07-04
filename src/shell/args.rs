//! 命令行参数解析类型。

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::str::FromStr;

use crate::shell::{events, productinfo};

const LONG_DESCRIPTION: &str = r"A bash-compatible, Rust-implemented shell.

This shell is distributed under the terms of the MIT license.";

const VERSION: &str = productinfo::PRODUCT_VERSION;

/// 标识 shell 要使用的输入后端。
#[derive(Clone, Copy)]
pub enum InputBackendType {
    /// 提供基础补全支持的输入后端。
    Basic,
    /// 最小化输入后端。
    Minimal,
}

impl FromStr for InputBackendType {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "basic" => Ok(Self::Basic),
            "minimal" => Ok(Self::Minimal),
            _ => Err(format!("unknown input backend '{value}'")),
        }
    }
}

/// 命令行解析错误。
#[derive(Clone, Debug)]
pub(crate) struct CliParseError {
    message: String,
    exit_code: i32,
}

impl CliParseError {
    fn new(message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            message: message.into(),
            exit_code,
        }
    }

    fn display_help() -> Self {
        Self::new(help_message(), 0)
    }

    fn display_version() -> Self {
        Self::new(format!("{} {VERSION}", productinfo::PRODUCT_NAME), 0)
    }

    fn missing_value(option: &str) -> Self {
        Self::new(format!("error: missing value for '{option}'"), 2)
    }

    fn unexpected_value(option: &str) -> Self {
        Self::new(format!("error: unexpected value for '{option}'"), 2)
    }

    fn unknown_argument(argument: &str) -> Self {
        Self::new(format!("error: unknown argument '{argument}'"), 2)
    }

    fn invalid_value(option: &str, value: &str, reason: impl std::fmt::Display) -> Self {
        Self::new(
            format!("error: invalid value '{value}' for '{option}': {reason}"),
            2,
        )
    }

    /// 打印错误或信息文本。
    pub(crate) fn print(&self) -> io::Result<()> {
        if self.exit_code == 0 {
            write_message(io::stdout(), &self.message)
        } else {
            write_message(io::stderr(), &self.message)
        }
    }

    /// 返回进程退出码。
    pub(crate) const fn exit_code(&self) -> i32 {
        self.exit_code
    }
}

impl std::fmt::Display for CliParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliParseError {}

/// shell 的已解析命令行参数。
#[derive(Clone)]
pub struct CommandLineArgs {
    /// 显示用法信息。
    pub help: Option<bool>,

    /// 显示 shell 版本。
    pub version: Option<bool>,

    /// TOML 配置文件路径, 会覆盖默认位置。
    pub config_file: Option<PathBuf>,

    /// 禁用 TOML 配置文件加载。
    pub no_config: bool,

    /// 启用 `noclobber` shell 选项。
    pub disallow_overwriting_regular_files_via_output_redirection: bool,

    /// 执行给定命令后退出。
    pub command: Option<String>,

    /// 启用命令非零退出时退出的行为。
    pub exit_on_nonzero_command_exit: bool,

    /// 禁用路径名展开, 也称为 filename globbing。
    pub disable_pathname_expansion: bool,

    /// 以交互模式运行。
    pub interactive: bool,

    /// 继承父进程注入的指定文件描述符。
    pub inherited_fds: Vec<i32>,

    /// 让 shell 表现为登录 shell。
    pub login: bool,

    /// 不执行命令。
    pub do_not_execute_commands: bool,

    /// 不使用 readline 输入。
    pub no_editing: bool,

    /// 不处理 profile/login 文件。
    pub no_profile: bool,

    /// 交互 shell 不处理 rc 文件。
    pub no_rc: bool,

    /// 不继承调用进程的环境变量。
    pub do_not_inherit_env: bool,

    /// 启用 `set -o` 选项。
    pub enabled_options: Vec<String>,

    /// 禁用 `set -o` 选项。
    pub disabled_options: Vec<String>,

    /// 启用 `shopt` 选项。
    pub enabled_shopt_options: Vec<String>,

    /// 禁用 `shopt` 选项。
    pub disabled_shopt_options: Vec<String>,

    /// 交互 shell 中要加载的 rc 文件路径。
    pub rc_file: Option<PathBuf>,

    /// 从标准输入读取命令。
    pub read_commands_from_stdin: bool,

    /// 只执行一条命令后退出。
    pub exit_after_one_command: bool,

    /// 将未设置变量的展开视为错误。
    pub treat_unset_variables_as_error: bool,

    /// 处理输入时打印输入。
    pub verbose: bool,

    /// 执行命令时打印命令和参数。
    pub print_commands_and_arguments: bool,

    /// 启用 xtrace 并写入给定文件。
    pub xtrace_file_path: Option<PathBuf>,

    /// 禁用彩色输出。
    pub disable_color: bool,

    /// 启用终端集成。
    pub terminal_shell_integration: bool,

    /// 启用 zsh 风格 preexec/precmd hook。
    pub zsh_style_hooks: bool,

    /// 输入后端。
    pub input_backend: Option<InputBackendType>,

    /// 启用指定事件类别的调试日志。
    pub enabled_debug_events: Vec<events::TraceEvent>,

    /// 禁用指定事件类别的日志。
    pub disabled_events: Vec<events::TraceEvent>,

    /// 要执行的脚本路径和参数。
    pub script_args: Vec<String>,
}

impl CommandLineArgs {
    /// 返回所有字段的默认值。
    #[must_use]
    pub fn default_values() -> Self {
        Self {
            help: None,
            version: None,
            config_file: None,
            no_config: false,
            disallow_overwriting_regular_files_via_output_redirection: false,
            command: None,
            exit_on_nonzero_command_exit: false,
            disable_pathname_expansion: false,
            interactive: false,
            inherited_fds: Vec::new(),
            login: false,
            do_not_execute_commands: false,
            no_editing: false,
            no_profile: false,
            no_rc: false,
            do_not_inherit_env: false,
            enabled_options: Vec::new(),
            disabled_options: Vec::new(),
            enabled_shopt_options: Vec::new(),
            disabled_shopt_options: Vec::new(),
            rc_file: None,
            read_commands_from_stdin: false,
            exit_after_one_command: false,
            treat_unset_variables_as_error: false,
            verbose: false,
            print_commands_and_arguments: false,
            xtrace_file_path: None,
            disable_color: false,
            terminal_shell_integration: false,
            zsh_style_hooks: false,
            input_backend: None,
            enabled_debug_events: Vec::new(),
            disabled_events: Vec::new(),
            script_args: Vec::new(),
        }
    }

    /// 解析命令行参数。
    pub(crate) fn try_parse_from(
        itr: impl IntoIterator<Item = String>,
    ) -> Result<Self, CliParseError> {
        let all_args: Vec<String> = itr.into_iter().collect();
        let args = if all_args.is_empty() {
            &[]
        } else {
            &all_args[1..]
        };
        let mut parsed = Self::default_values();
        let mut index = 0;

        while index < args.len() {
            let argument = &args[index];

            if argument == "--" {
                parsed.script_args.extend(args[index..].iter().cloned());
                break;
            }

            if is_plus_option(argument) {
                index = parse_plus_option(&mut parsed, args, index)?;
                continue;
            }

            if let Some(option) = argument.strip_prefix("--") {
                index = parse_long_option(&mut parsed, args, index, option)?;
                continue;
            }

            if is_short_option(argument) {
                index = parse_short_option_group(&mut parsed, args, index)?;
                continue;
            }

            parsed.script_args.extend(args[index..].iter().cloned());
            break;
        }

        Ok(parsed)
    }

    /// 返回参数是否表示 shell 应该以交互模式运行。
    pub fn is_interactive(&self) -> bool {
        if self.interactive {
            return true;
        }

        if self.command.is_some() || !self.script_args.is_empty() {
            return false;
        }

        if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
            return false;
        }

        true
    }
}

fn parse_long_option(
    parsed: &mut CommandLineArgs,
    args: &[String],
    index: usize,
    option: &str,
) -> Result<usize, CliParseError> {
    let (name, inline_value) = split_long_option(option);
    match name {
        "help" => {
            reject_inline_value(name, inline_value)?;
            Err(CliParseError::display_help())
        }
        "version" => {
            reject_inline_value(name, inline_value)?;
            Err(CliParseError::display_version())
        }
        "config" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            parsed.config_file = Some(PathBuf::from(value));
            Ok(next_index)
        }
        "no-config" => {
            reject_inline_value(name, inline_value)?;
            parsed.no_config = true;
            Ok(index + 1)
        }
        "inherit-fd" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            let fd = value
                .parse::<i32>()
                .map_err(|err| CliParseError::invalid_value("--inherit-fd", &value, err))?;
            parsed.inherited_fds.push(fd);
            Ok(next_index)
        }
        "login" => {
            reject_inline_value(name, inline_value)?;
            parsed.login = true;
            Ok(index + 1)
        }
        "noediting" => {
            reject_inline_value(name, inline_value)?;
            parsed.no_editing = true;
            Ok(index + 1)
        }
        "noprofile" => {
            reject_inline_value(name, inline_value)?;
            parsed.no_profile = true;
            Ok(index + 1)
        }
        "norc" => {
            reject_inline_value(name, inline_value)?;
            parsed.no_rc = true;
            Ok(index + 1)
        }
        "noenv" => {
            reject_inline_value(name, inline_value)?;
            parsed.do_not_inherit_env = true;
            Ok(index + 1)
        }
        "+o" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            parsed.disabled_options.push(value);
            Ok(next_index)
        }
        "+O" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            parsed.disabled_shopt_options.push(value);
            Ok(next_index)
        }
        "rcfile" | "init-file" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            parsed.rc_file = Some(PathBuf::from(value));
            Ok(next_index)
        }
        "verbose" => {
            reject_inline_value(name, inline_value)?;
            parsed.verbose = true;
            Ok(index + 1)
        }
        "xtrace-file" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            parsed.xtrace_file_path = Some(PathBuf::from(value));
            Ok(next_index)
        }
        "disable-color" => {
            reject_inline_value(name, inline_value)?;
            parsed.disable_color = true;
            Ok(index + 1)
        }
        "enable-terminal-integration" => {
            reject_inline_value(name, inline_value)?;
            parsed.terminal_shell_integration = true;
            Ok(index + 1)
        }
        "enable-zsh-hooks" => {
            reject_inline_value(name, inline_value)?;
            parsed.zsh_style_hooks = true;
            Ok(index + 1)
        }
        "input-backend" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            let input_backend = value
                .parse::<InputBackendType>()
                .map_err(|err| CliParseError::invalid_value("--input-backend", &value, err))?;
            parsed.input_backend = Some(input_backend);
            Ok(next_index)
        }
        "debug" | "log-enable" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            let event = value
                .parse::<events::TraceEvent>()
                .map_err(|err| CliParseError::invalid_value("--debug", &value, err))?;
            parsed.enabled_debug_events.push(event);
            Ok(next_index)
        }
        "disable-event" | "log-disable" => {
            let (value, next_index) = take_long_value(args, index, name, inline_value)?;
            let event = value
                .parse::<events::TraceEvent>()
                .map_err(|err| CliParseError::invalid_value("--disable-event", &value, err))?;
            parsed.disabled_events.push(event);
            Ok(next_index)
        }
        _ => Err(CliParseError::unknown_argument(&format!("--{name}"))),
    }
}

fn parse_plus_option(
    parsed: &mut CommandLineArgs,
    args: &[String],
    index: usize,
) -> Result<usize, CliParseError> {
    let argument = &args[index];
    let Some(option) = argument.strip_prefix('+') else {
        return Err(CliParseError::unknown_argument(argument));
    };
    let mut chars = option.chars();
    let Some(flag) = chars.next() else {
        return Err(CliParseError::unknown_argument(argument));
    };
    let rest = chars.as_str();
    let attached = (!rest.is_empty()).then_some(rest);

    match flag {
        'o' => {
            let (value, next_index) = take_short_value(args, index, "+o", attached)?;
            parsed.disabled_options.push(value);
            Ok(next_index)
        }
        'O' => {
            let (value, next_index) = take_short_value(args, index, "+O", attached)?;
            parsed.disabled_shopt_options.push(value);
            Ok(next_index)
        }
        _ => Err(CliParseError::unknown_argument(argument)),
    }
}

fn parse_short_option_group(
    parsed: &mut CommandLineArgs,
    args: &[String],
    index: usize,
) -> Result<usize, CliParseError> {
    let argument = &args[index];
    let Some(mut flags) = argument.strip_prefix('-') else {
        return Err(CliParseError::unknown_argument(argument));
    };

    while let Some(flag) = flags.chars().next() {
        flags = &flags[flag.len_utf8()..];
        match flag {
            'C' => parsed.disallow_overwriting_regular_files_via_output_redirection = true,
            'c' => {
                let attached = (!flags.is_empty()).then_some(flags);
                return parse_command_value(parsed, args, index, attached);
            }
            'e' => parsed.exit_on_nonzero_command_exit = true,
            'f' => parsed.disable_pathname_expansion = true,
            'i' => parsed.interactive = true,
            'l' => parsed.login = true,
            'n' => parsed.do_not_execute_commands = true,
            'o' => {
                let attached = (!flags.is_empty()).then_some(flags);
                let (value, next_index) = take_short_value(args, index, "-o", attached)?;
                parsed.enabled_options.push(value);
                return Ok(next_index);
            }
            'O' => {
                let attached = (!flags.is_empty()).then_some(flags);
                let (value, next_index) = take_short_value(args, index, "-O", attached)?;
                parsed.enabled_shopt_options.push(value);
                return Ok(next_index);
            }
            's' => parsed.read_commands_from_stdin = true,
            't' => parsed.exit_after_one_command = true,
            'u' => parsed.treat_unset_variables_as_error = true,
            'v' => parsed.verbose = true,
            'x' => parsed.print_commands_and_arguments = true,
            _ => return Err(CliParseError::unknown_argument(&format!("-{flag}"))),
        }
    }

    Ok(index + 1)
}

fn parse_command_value(
    parsed: &mut CommandLineArgs,
    args: &[String],
    index: usize,
    attached: Option<&str>,
) -> Result<usize, CliParseError> {
    if let Some(value) = attached {
        parsed.command = Some(value.to_string());
        parsed.script_args.extend(args[index + 1..].iter().cloned());
        return Ok(args.len());
    }

    let Some(value) = args.get(index + 1) else {
        return Err(CliParseError::missing_value("-c"));
    };

    if value == "--" {
        let Some(command) = args.get(index + 2) else {
            return Err(CliParseError::missing_value("-c"));
        };
        parsed.command = Some(command.clone());
        parsed.script_args.extend(args[index + 3..].iter().cloned());
    } else {
        parsed.command = Some(value.clone());
        parsed.script_args.extend(args[index + 2..].iter().cloned());
    }

    Ok(args.len())
}

fn split_long_option(option: &str) -> (&str, Option<&str>) {
    option
        .split_once('=')
        .map_or((option, None), |(name, value)| (name, Some(value)))
}

fn reject_inline_value(name: &str, inline_value: Option<&str>) -> Result<(), CliParseError> {
    if inline_value.is_some() {
        Err(CliParseError::unexpected_value(&format!("--{name}")))
    } else {
        Ok(())
    }
}

fn take_long_value(
    args: &[String],
    index: usize,
    name: &str,
    inline_value: Option<&str>,
) -> Result<(String, usize), CliParseError> {
    if let Some(value) = inline_value {
        return Ok((value.to_string(), index + 1));
    }

    let option = format!("--{name}");
    let Some(value) = args.get(index + 1) else {
        return Err(CliParseError::missing_value(&option));
    };
    if value == "--" {
        return Err(CliParseError::missing_value(&option));
    }

    Ok((value.clone(), index + 2))
}

fn take_short_value(
    args: &[String],
    index: usize,
    option: &str,
    attached: Option<&str>,
) -> Result<(String, usize), CliParseError> {
    if let Some(value) = attached {
        return Ok((value.to_string(), index + 1));
    }

    let Some(value) = args.get(index + 1) else {
        return Err(CliParseError::missing_value(option));
    };
    if value == "--" {
        return Err(CliParseError::missing_value(option));
    }

    Ok((value.clone(), index + 2))
}

fn is_short_option(argument: &str) -> bool {
    argument.starts_with('-') && argument != "-" && !argument.starts_with("--")
}

fn is_plus_option(argument: &str) -> bool {
    matches!(argument.as_bytes(), [b'+', b'o', ..] | [b'+', b'O', ..])
}

fn help_message() -> String {
    let parser = nanoargs::ArgBuilder::new()
        .name(productinfo::PRODUCT_NAME)
        .description(LONG_DESCRIPTION)
        .version(VERSION)
        .flag(nanoargs::Flag::new("help").desc("Display usage information"))
        .flag(nanoargs::Flag::new("version").desc("Display shell version"))
        .option(
            nanoargs::Opt::new("config")
                .placeholder("FILE")
                .desc("Use a TOML config file"),
        )
        .flag(nanoargs::Flag::new("no-config").desc("Disable config loading"))
        .option(
            nanoargs::Opt::new("input-backend")
                .placeholder("BACKEND")
                .desc("Select input backend: basic or minimal"),
        )
        .positional(
            nanoargs::Pos::new("SCRIPT_PATH [SCRIPT_ARGS]...")
                .desc("Path and arguments for script to execute")
                .multi(),
        )
        .build()
        .expect("static nanoargs schema should be valid");

    let mut message = parser.help_text();
    message.push_str("\nBash-compatible short options are also supported: -C -c -e -f -i -l -n -o -O -s -t -u -v -x.\n");
    message.push_str("Use +o/+O or --+o/--+O to disable set/shopt options.\n");
    message
}

fn write_message(mut writer: impl Write, message: &str) -> io::Result<()> {
    writer.write_all(message.as_bytes())?;
    if !message.ends_with('\n') {
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_values() {
        let args = CommandLineArgs::default_values();
        assert!(!args.interactive);
        assert!(!args.login);
        assert!(args.command.is_none());
        assert!(args.script_args.is_empty());
    }
}
