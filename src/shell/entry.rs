//! Implements the command-line interface for the shell.

use crate::shell::args::CommandLineArgs;
use crate::shell::args::InputBackendType;

use crate::shell::error_formatter;
use crate::shell::events;
use crate::shell::productinfo;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex as StdMutex};

#[allow(unused_imports, reason = "only used in some configs")]
use std::io::IsTerminal;

static TRACE_EVENT_CONFIG: LazyLock<Arc<StdMutex<Option<events::TraceEventConfig>>>> =
    LazyLock::new(|| Arc::new(StdMutex::new(None)));

type BashShell = crate::engine::Shell;

/// Shell 启动时要执行的顶层模式。
#[derive(Clone, Debug)]
pub(crate) enum ShellRunMode {
    /// 执行 `-c` 传入的命令字符串。
    CommandString(String),
    /// 从标准输入读取命令。
    Stdin,
    /// 执行脚本文件和其参数。
    Script { path: String, args: Vec<String> },
    /// 启动交互式读取循环。
    Interactive,
}

/// Shell 启动计划, 集中保存由命令行参数推导出的启动决策。
#[derive(Clone)]
pub(crate) struct ShellRunPlan {
    /// 启动时要执行的顶层模式。
    pub mode: ShellRunMode,
    /// shell 本身是否按交互 shell 初始化。
    pub interactive_shell: bool,
    /// 是否启动真正的交互 session, 包含 prompt, history 和 PROMPT_COMMAND。
    pub interactive_session: bool,
    /// 未显式指定 input backend 时使用的默认 backend。
    pub default_input_backend: InputBackendType,
}

impl ShellRunPlan {
    /// 从命令行参数构建启动计划。
    pub(crate) fn from_args(args: &CommandLineArgs) -> Self {
        Self::from_args_with_stdin_terminal(args, std::io::stdin().is_terminal())
    }

    fn from_args_with_stdin_terminal(args: &CommandLineArgs, stdin_is_terminal: bool) -> Self {
        let mode = if let Some(command) = &args.command {
            ShellRunMode::CommandString(command.clone())
        } else if args.read_commands_from_stdin {
            ShellRunMode::Stdin
        } else if let Some(path) = args.script_args.first() {
            ShellRunMode::Script {
                path: path.clone(),
                args: args.script_args.iter().skip(1).cloned().collect(),
            }
        } else {
            ShellRunMode::Interactive
        };

        let uses_interactive_input_loop = match &mode {
            ShellRunMode::CommandString(_) | ShellRunMode::Script { .. } => false,
            ShellRunMode::Stdin | ShellRunMode::Interactive => true,
        };
        let default_input_backend = if stdin_is_terminal && uses_interactive_input_loop {
            InputBackendType::Basic
        } else {
            InputBackendType::Minimal
        };

        let interactive_session = matches!(mode, ShellRunMode::Interactive);

        Self {
            mode,
            interactive_shell: args.is_interactive(),
            interactive_session,
            default_input_backend,
        }
    }
}

impl std::fmt::Debug for ShellRunPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellRunPlan")
            .field("mode", &self.mode)
            .field("interactive_shell", &self.interactive_shell)
            .field("interactive_session", &self.interactive_session)
            .field(
                "default_input_backend",
                &InputBackendTypeDebug(self.default_input_backend),
            )
            .finish()
    }
}

struct InputBackendTypeDebug(InputBackendType);

impl std::fmt::Debug for InputBackendTypeDebug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            InputBackendType::Basic => f.write_str("Basic"),
            InputBackendType::Minimal => f.write_str("Minimal"),
        }
    }
}

/// Runs the shell according to the provided command-line arguments.
/// Also responsible for loading profiles and rc files as appropriate.
///
/// # Arguments
///
/// * `shell_ref` - A reference to the shell to run.
/// * `args` - The parsed command-line arguments.
/// * `input_backend` - The input backend to use.
/// * `ui_options` - The user interface options to use.
pub(crate) async fn run_in_shell(
    shell_ref: &crate::interactive::ShellRef,
    args: CommandLineArgs,
    input_backend: &mut impl crate::interactive::InputBackend,
    ui_options: &crate::interactive::UIOptions,
) -> Result<u8, crate::interactive::ShellError> {
    let plan = ShellRunPlan::from_args(&args);

    // First load profile and rc files as appropriate.
    initialize_shell(shell_ref, &args).await?;

    match plan.mode {
        // If a command was specified via -c, then run that command and then exit.
        ShellRunMode::CommandString(command) => {
            shell_ref.lock().await.run_dash_c_command(command).await?;
        }

        // If -s was provided, then read commands from stdin. If there was a script (and optionally
        // args) passed on the command line via positional arguments, then we copy over the
        // parameters but do *not* execute it.
        ShellRunMode::Stdin => {
            let interactive_options = crate::interactive::UIOptions::stdin_input_loop();
            crate::interactive::InteractiveShell::new(
                shell_ref,
                input_backend,
                &interactive_options,
            )?
            .run_stdin_input_loop()
            .await?;
        }

        // If a script path was provided, then run the script.
        ShellRunMode::Script { path, args } => {
            shell_ref
                .lock()
                .await
                .run_script(Path::new(&path), args.iter())
                .await?;
        }

        // If we got down here, then we don't have any commands to run. We'll be reading
        // them in from stdin one way or the other.
        ShellRunMode::Interactive => {
            crate::interactive::InteractiveShell::new(shell_ref, input_backend, ui_options)?
                .run_interactively()
                .await?;
        }
    }

    // Make sure to return the last result observed in the shell.
    let result = shell_ref.lock().await.last_exit_status();

    Ok(result)
}

/// Initializes a shell by loading profile and rc files as appropriate.
///
/// # Arguments
///
/// * `shell_ref` - A reference to the shell to initialize.
/// * `args` - The parsed command-line arguments.
async fn initialize_shell(
    shell_ref: &crate::interactive::ShellRef,
    args: &CommandLineArgs,
) -> Result<(), crate::interactive::ShellError> {
    // Compute desired profile-loading behavior.
    let profile = if args.no_profile {
        crate::engine::ProfileLoadBehavior::Skip
    } else {
        crate::engine::ProfileLoadBehavior::LoadDefault
    };

    // Compute desired rc-loading behavior.
    let rc = if args.no_rc {
        crate::engine::RcLoadBehavior::Skip
    } else if let Some(rc_file) = &args.rc_file {
        crate::engine::RcLoadBehavior::LoadCustom(rc_file.clone())
    } else {
        crate::engine::RcLoadBehavior::LoadDefault
    };

    shell_ref.lock().await.load_config(&profile, &rc).await?;

    Ok(())
}

/// Instantiates a shell from command-line arguments. Does *not* run any code in the shell.
///
/// # Arguments
///
/// * `args` - The parsed command-line arguments.
/// * `cli_args` - The raw command-line arguments.
pub(crate) async fn instantiate_shell(
    args: &CommandLineArgs,
    cli_args: Vec<String>,
) -> Result<BashShell, crate::interactive::ShellError> {
    instantiate_shell_from_args(args, cli_args).await
}

/// Instantiates a shell from command-line arguments. Does *not* run any code in the shell.
///
/// # Arguments
///
/// * `args` - The parsed command-line arguments.
/// * `cli_args` - The raw command-line arguments.
async fn instantiate_shell_from_args(
    args: &CommandLineArgs,
    cli_args: Vec<String>,
) -> Result<BashShell, crate::interactive::ShellError> {
    let plan = ShellRunPlan::from_args(args);

    // Compute login flag.
    let login = args.login || cli_args.first().is_some_and(|argv0| argv0.starts_with('-'));

    // Compute shell name.
    let shell_name = if args.command.is_some() && !args.script_args.is_empty() {
        Some(args.script_args[0].clone())
    } else if !cli_args.is_empty() {
        Some(cli_args[0].clone())
    } else {
        None
    };

    // Compute positional shell arguments.
    let shell_args = if args.command.is_some() {
        Some(args.script_args.iter().skip(1).cloned().collect())
    } else if args.read_commands_from_stdin {
        Some(args.script_args.clone())
    } else {
        None
    };

    // Commands are read from stdin if -s was provided, or if no command was specified (either via
    // -c or as a positional argument).
    let read_commands_from_stdin = (args.read_commands_from_stdin && args.command.is_none())
        || (args.script_args.is_empty() && args.command.is_none());

    // Identify the file descriptors to inherit.
    let fds = args
        .inherited_fds
        .iter()
        .filter_map(|&fd| {
            crate::engine::sys::fd::try_get_file_for_open_fd(fd).map(|file| (fd, file))
        })
        .collect();

    // Set up the shell builder with the requested options.
    // NOTE: We skip loading profile and rc files here; that will be handled later after we've
    // fully instantiated everything we want set before running any code.
    let shell = crate::engine::Shell::builder()
        .disable_options(args.disabled_options.clone())
        .disable_shopt_options(args.disabled_shopt_options.clone())
        .disallow_overwriting_regular_files_via_output_redirection(
            args.disallow_overwriting_regular_files_via_output_redirection,
        )
        .enable_options(args.enabled_options.clone())
        .enable_shopt_options(args.enabled_shopt_options.clone())
        .do_not_execute_commands(args.do_not_execute_commands)
        .exit_after_one_command(args.exit_after_one_command)
        .login(login)
        .interactive(plan.interactive_shell)
        .command_string_mode(args.command.is_some())
        .no_editing(args.no_editing)
        .profile(crate::engine::ProfileLoadBehavior::Skip)
        .rc(crate::engine::RcLoadBehavior::Skip)
        .do_not_inherit_env(args.do_not_inherit_env)
        .fds(fds)
        .maybe_shell_args(shell_args)
        .print_commands_and_arguments(args.print_commands_and_arguments)
        .read_commands_from_stdin(read_commands_from_stdin)
        .maybe_shell_name(shell_name)
        .shell_product_display_str(productinfo::get_product_display_str())
        .treat_unset_variables_as_error(args.treat_unset_variables_as_error)
        .exit_on_nonzero_command_exit(args.exit_on_nonzero_command_exit)
        .disable_pathname_expansion(args.disable_pathname_expansion)
        .verbose(args.verbose)
        .error_formatter(new_error_behavior(args))
        .shell_version(env!("CARGO_PKG_VERSION").to_string())
        .builtins(crate::builtins::default_builtins());

    // Build the shell.
    let mut shell = shell.build().await?;

    // Make adjustments.
    if let Some(xtrace_file_path) = &args.xtrace_file_path {
        enable_xtrace_to_file(&mut shell, xtrace_file_path)?;
    }

    Ok(shell)
}

fn enable_xtrace_to_file(
    shell: &mut crate::engine::Shell,
    file_path: &Path,
) -> Result<(), crate::interactive::ShellError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(file_path)
        .map_err(|e| {
            crate::interactive::ShellError::FailedToCreateXtraceFile(file_path.to_path_buf(), e)
        })?;

    let file = crate::engine::openfiles::OpenFile::from(file);
    let file_fd = shell.open_files_mut().add(file)?;

    shell.options_mut().print_commands_and_arguments = true;
    shell.set_env_global(
        "BASH_XTRACEFD",
        crate::engine::ShellVariable::new(file_fd.to_string()),
    )?;

    Ok(())
}

fn new_error_behavior(args: &CommandLineArgs) -> Arc<dyn crate::engine::ErrorFormatter> {
    Arc::new(error_formatter::Formatter {
        use_color: !args.disable_color,
    })
}

pub(crate) fn get_default_input_backend_type(args: &CommandLineArgs) -> InputBackendType {
    ShellRunPlan::from_args(args).default_input_backend
}

pub(crate) fn get_event_config() -> Arc<StdMutex<Option<events::TraceEventConfig>>> {
    TRACE_EVENT_CONFIG.clone()
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)]
mod tests {
    use super::*;
    use anyhow::Result;
    use pretty_assertions::{assert_eq, assert_matches};

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    fn assert_basic_backend(input_backend: InputBackendType) {
        assert!(matches!(input_backend, InputBackendType::Basic));
    }

    fn assert_minimal_backend(input_backend: InputBackendType) {
        assert!(matches!(input_backend, InputBackendType::Minimal));
    }

    #[test]
    fn shell_run_plan_for_command_string() -> Result<()> {
        let parsed_args = CommandLineArgs::try_parse_from(args(&["bash", "-c", "echo hi"]))?;
        let plan = ShellRunPlan::from_args_with_stdin_terminal(&parsed_args, true);

        assert_matches!(&plan.mode, ShellRunMode::CommandString(command) if command == "echo hi");
        assert_eq!(plan.interactive_shell, parsed_args.is_interactive());
        assert!(!plan.interactive_session);
        assert_minimal_backend(plan.default_input_backend);
        Ok(())
    }

    #[test]
    fn shell_run_plan_for_stdin() -> Result<()> {
        let parsed_args = CommandLineArgs::try_parse_from(args(&["bash", "-s", "arg0", "arg1"]))?;
        let plan = ShellRunPlan::from_args_with_stdin_terminal(&parsed_args, true);

        assert_matches!(&plan.mode, ShellRunMode::Stdin);
        assert_eq!(plan.interactive_shell, parsed_args.is_interactive());
        assert!(!plan.interactive_session);
        assert_basic_backend(plan.default_input_backend);
        Ok(())
    }

    #[test]
    fn shell_run_plan_for_script() -> Result<()> {
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "script.sh", "one", "two"]))?;
        let plan = ShellRunPlan::from_args_with_stdin_terminal(&parsed_args, true);

        match &plan.mode {
            ShellRunMode::Script { path, args } => {
                assert_eq!(path, "script.sh");
                assert_eq!(args, &vec!["one".to_string(), "two".to_string()]);
            }
            mode => panic!("unexpected shell run mode: {mode:?}"),
        }
        assert_eq!(plan.interactive_shell, parsed_args.is_interactive());
        assert!(!plan.interactive_session);
        assert_minimal_backend(plan.default_input_backend);
        Ok(())
    }

    #[test]
    fn shell_run_plan_for_empty_args() -> Result<()> {
        let parsed_args = CommandLineArgs::try_parse_from(args(&["bash"]))?;
        let terminal_plan = ShellRunPlan::from_args_with_stdin_terminal(&parsed_args, true);
        let non_terminal_plan = ShellRunPlan::from_args_with_stdin_terminal(&parsed_args, false);

        assert_matches!(&terminal_plan.mode, ShellRunMode::Interactive);
        assert_eq!(
            terminal_plan.interactive_shell,
            parsed_args.is_interactive()
        );
        assert!(terminal_plan.interactive_session);
        assert_basic_backend(terminal_plan.default_input_backend);

        assert_matches!(&non_terminal_plan.mode, ShellRunMode::Interactive);
        assert_eq!(
            non_terminal_plan.interactive_shell,
            parsed_args.is_interactive()
        );
        assert!(non_terminal_plan.interactive_session);
        assert_minimal_backend(non_terminal_plan.default_input_backend);
        Ok(())
    }

    #[test]
    fn parse_empty_args() -> Result<()> {
        let parsed_args = CommandLineArgs::try_parse_from(args(&["bash"]))?;
        assert_matches!(parsed_args.script_args.as_slice(), []);
        Ok(())
    }

    #[test]
    fn parse_script_and_args() -> Result<()> {
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "some-script", "-x", "1", "--option"]))?;
        assert_eq!(
            parsed_args.script_args,
            ["some-script", "-x", "1", "--option"]
        );
        Ok(())
    }

    #[test]
    fn parse_script_and_args_with_double_dash_in_script_args() -> Result<()> {
        let parsed_args = CommandLineArgs::try_parse_from(args(&["bash", "some-script", "--"]))?;
        assert_eq!(parsed_args.script_args, ["some-script", "--"]);
        Ok(())
    }

    #[test]
    fn parse_unknown_args() {
        let result = CommandLineArgs::try_parse_from(args(&["bash", "--unknown-option"]));
        assert!(result.is_err());
    }

    #[test]
    fn parse_c_with_double_dash_separator() -> Result<()> {
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "-c", "--", "echo hello", "arg0"]))?;
        assert_eq!(parsed_args.command, Some("echo hello".to_string()));
        assert_eq!(parsed_args.script_args, ["arg0"]);
        Ok(())
    }

    #[test]
    fn parse_c_with_double_dash_no_command() {
        assert!(CommandLineArgs::try_parse_from(args(&["bash", "-c", "--"])).is_err());
    }

    #[test]
    fn parse_c_with_double_dash_command_is_double_dash() -> Result<()> {
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "-c", "--", "--", "echo", "hi"]))?;
        assert_eq!(parsed_args.command, Some("--".to_string()));
        assert_eq!(parsed_args.script_args, ["echo", "hi"]);
        Ok(())
    }

    #[test]
    fn parse_ec_with_double_dash_separator() -> Result<()> {
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "-ec", "--", "echo hello", "arg0"]))?;
        assert_eq!(parsed_args.command, Some("echo hello".to_string()));
        assert!(parsed_args.exit_on_nonzero_command_exit);
        assert_eq!(parsed_args.script_args, ["arg0"]);
        Ok(())
    }

    #[test]
    fn parse_c_with_value_before_double_dash_unchanged() -> Result<()> {
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "-c", "echo hi", "--", "arg0"]))?;
        assert_eq!(parsed_args.command, Some("echo hi".to_string()));
        assert_eq!(parsed_args.script_args, ["--", "arg0"]);
        Ok(())
    }

    #[test]
    fn parse_o_with_double_dash_is_not_transformed() {
        // Unlike -c, bash's -o consumes -- as its literal value (invalid option
        // name), not as an option terminator. Verify we don't transform it.
        let result = CommandLineArgs::try_parse_from(args(&["bash", "-o", "--"]));
        // 这里 -o 不能把 -- 当作 -c 那样的分隔符, 因此缺少值并解析失败。
        // 关键断言是不能把 -- 后面的参数重新解释成 -o 的值。
        assert!(result.is_err());
    }

    #[test]
    fn parse_oc_not_treated_as_pending_c() -> Result<()> {
        // -oc means -o with value "c", not -o flag + -c flag. The --
        // should NOT be treated as an option terminator for -c.
        let parsed_args = CommandLineArgs::try_parse_from(args(&["bash", "-oc", "--", "echo"]))?;
        // -o consumed "c" as its value; -- split the rest; no -c command.
        assert!(parsed_args.command.is_none());
        assert_eq!(parsed_args.script_args, ["--", "echo"]);
        Ok(())
    }

    #[test]
    fn parse_bool_flag_before_double_dash_not_transformed() -> Result<()> {
        // -e is a boolean flag, not -c. The -- should NOT be removed;
        // everything from -- onward becomes positional (including -c).
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "-e", "--", "-c", "echo"]))?;
        assert!(parsed_args.command.is_none());
        assert!(parsed_args.exit_on_nonzero_command_exit);
        assert_eq!(parsed_args.script_args, ["--", "-c", "echo"]);
        Ok(())
    }

    #[test]
    fn parse_c_with_double_dash_and_later_double_dash() -> Result<()> {
        // 第一个 -- 作为 -c 的特殊分隔符被消费, 第二个 -- 保留在 script_args 中。
        let parsed_args =
            CommandLineArgs::try_parse_from(args(&["bash", "-c", "--", "echo", "--", "more"]))?;
        assert_eq!(parsed_args.command, Some("echo".to_string()));
        assert_eq!(parsed_args.script_args, ["--", "more"]);
        Ok(())
    }
}
