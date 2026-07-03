//! Command execution

use std::{
    borrow::Cow,
    ffi::OsStr,
    fmt::Display,
    path::{Path, PathBuf},
    process::Stdio,
};

use crate::engine::sys::traits::PathExt;
use crate::parser::ast;
use itertools::Itertools;
use sys::commands::{CommandExt, CommandFdInjectionExt, CommandFgControlExt};

use crate::engine::{
    ErrorKind, ExecutionControlFlow, ExecutionParameters, ExecutionResult, Shell, ShellFd,
    builtins, commands, env, error, escape, functions,
    interp::{self, Execute, ProcessGroupPolicy},
    openfiles::{self, OpenFile, OpenFiles},
    pathsearch, processes,
    results::ExecutionSpawnResult,
    sys, trace_categories, traps, variables,
};

/// 控制命令解析器要查询哪些命名空间以及如何查询 PATH.
#[derive(Clone, Debug)]
pub struct ResolveOptions {
    /// 是否查询别名.
    pub include_aliases: bool,
    /// 是否查询 shell 关键字.
    pub include_keywords: bool,
    /// 是否查询 shell 函数.
    pub include_functions: bool,
    /// 是否查询内置命令.
    pub include_builtins: bool,
    /// 是否允许返回禁用状态的内置命令.
    pub include_disabled_builtins: bool,
    /// 是否查询可执行文件路径.
    pub include_path: bool,
    /// 查询 PATH 时是否先包含哈希缓存结果.
    pub include_hashed: bool,
    /// 是否返回所有匹配位置, 而不是第一个匹配.
    pub all_locations: bool,
    /// 是否跳过别名, 关键字, 函数和内置命令, 只查文件路径.
    pub force_path_search: bool,
    /// 是否使用系统默认工具 PATH, 而不是 shell 的 PATH.
    pub use_default_path: bool,
    /// 路径含分隔符时是否返回输入字面值, 而不是绝对路径.
    pub literal_path_with_separator: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            include_aliases: true,
            include_keywords: true,
            include_functions: true,
            include_builtins: true,
            include_disabled_builtins: false,
            include_path: true,
            include_hashed: true,
            all_locations: false,
            force_path_search: false,
            use_default_path: false,
            literal_path_with_separator: false,
        }
    }
}

/// 命令解析结果, 按 shell 命名空间描述命令来源.
pub enum ResolvedCommand<'a> {
    /// 命令解析为别名, 内容为别名替换文本.
    Alias(&'a str),
    /// 命令解析为 shell 关键字.
    Keyword,
    /// 命令解析为 shell 函数定义.
    Function(&'a crate::parser::ast::FunctionDefinition),
    /// 命令解析为启用状态的内置命令.
    Builtin,
    /// 命令解析为外部可执行文件.
    External {
        /// 可执行文件路径.
        path: PathBuf,
        /// 该结果是否来自命令哈希缓存.
        hashed: bool,
    },
}

/// 按 bash 风格的命令命名空间顺序解析命令名.
pub fn resolve_command<'a>(
    shell: &'a Shell,
    name: &str,
    options: &ResolveOptions,
) -> Vec<ResolvedCommand<'a>> {
    let mut resolved = Vec::new();

    if !options.force_path_search {
        if options.include_aliases
            && let Some(alias) = shell.aliases().get(name)
        {
            resolved.push(ResolvedCommand::Alias(alias));
            if !options.all_locations {
                return resolved;
            }
        }

        if options.include_keywords && shell.is_keyword(name) {
            resolved.push(ResolvedCommand::Keyword);
            if !options.all_locations {
                return resolved;
            }
        }

        if options.include_functions
            && let Some(registration) = shell.funcs().get(name)
        {
            resolved.push(ResolvedCommand::Function(registration.definition()));
            if !options.all_locations {
                return resolved;
            }
        }

        if options.include_builtins
            && shell
                .builtins()
                .get(name)
                .is_some_and(|b| options.include_disabled_builtins || !b.disabled)
        {
            resolved.push(ResolvedCommand::Builtin);
            if !options.all_locations {
                return resolved;
            }
        }
    }

    if !options.include_path {
        return resolved;
    }

    if sys::fs::contains_path_separator(name) {
        let candidate_path = shell.absolute_path(Path::new(name));
        if candidate_path.executable() {
            resolved.push(ResolvedCommand::External {
                path: if options.literal_path_with_separator {
                    PathBuf::from(name)
                } else {
                    candidate_path
                },
                hashed: false,
            });
        }

        return resolved;
    }

    if options.use_default_path {
        let default_paths = sys::fs::get_default_standard_utils_paths();
        for path in pathsearch::search_for_executable(default_paths.iter(), name) {
            resolved.push(ResolvedCommand::External {
                path,
                hashed: false,
            });
            if !options.all_locations {
                return resolved;
            }
        }
    } else {
        if options.include_hashed
            && let Some(path) = shell.program_location_cache().get(name)
        {
            resolved.push(ResolvedCommand::External { path, hashed: true });
            if !options.all_locations {
                return resolved;
            }
        }

        for path in shell.find_executables_in_path(name) {
            resolved.push(ResolvedCommand::External {
                path,
                hashed: false,
            });
            if !options.all_locations {
                return resolved;
            }
        }
    }

    resolved
}

/// Encapsulates the result of waiting for a command to complete.
pub enum CommandWaitResult {
    /// The command completed.
    CommandCompleted(ExecutionResult),
    /// The command was stopped before it completed.
    CommandStopped(ExecutionResult, processes::ChildProcess),
}

/// Represents the context for executing a command.
pub struct ExecutionContext<'a> {
    /// The shell in which the command is being executed.
    pub shell: &'a mut Shell,
    /// The name of the command being executed.
    pub command_name: String,
    /// The parameters for the execution.
    pub params: ExecutionParameters,
}

impl ExecutionContext<'_> {
    /// Returns the standard input file; usable with `write!` et al.
    pub fn stdin(&self) -> impl std::io::Read + 'static {
        self.params.stdin(self.shell)
    }

    /// Returns the standard output file; usable with `write!` et al.
    pub fn stdout(&self) -> impl std::io::Write + 'static {
        self.params.stdout(self.shell)
    }

    /// Returns the standard error file; usable with `write!` et al.
    pub fn stderr(&self) -> impl std::io::Write + 'static {
        self.params.stderr(self.shell)
    }

    /// Returns the file descriptor with the given number. Returns `None`
    /// if the file descriptor is not open.
    ///
    /// # Arguments
    ///
    /// * `fd` - The file descriptor number to retrieve.
    pub fn try_fd(&self, fd: ShellFd) -> Option<openfiles::OpenFile> {
        self.params.try_fd(self.shell, fd)
    }

    /// Iterates over all open file descriptors.
    pub fn iter_fds(&self) -> impl Iterator<Item = (ShellFd, openfiles::OpenFile)> {
        self.params.iter_fds(self.shell)
    }
}

/// An argument to a command.
#[derive(Clone, Debug)]
pub enum CommandArg {
    /// A simple string argument.
    String(String),
    /// An assignment/declaration; typically treated as a string, but will
    /// be specially handled by a limited set of built-in commands.
    Assignment(ast::Assignment),
}

impl Display for CommandArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::String(s) => f.write_str(s),
            Self::Assignment(a) => write!(f, "{a}"),
        }
    }
}

impl From<String> for CommandArg {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&String> for CommandArg {
    fn from(value: &String) -> Self {
        Self::String(value.clone())
    }
}

impl From<&str> for CommandArg {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl CommandArg {
    /// 将命令参数转换为普通字符串.
    pub(crate) fn into_string(self) -> String {
        match self {
            Self::String(s) => s,
            Self::Assignment(a) => a.to_string(),
        }
    }

    pub(crate) fn quote_for_tracing(&self) -> Cow<'_, str> {
        match self {
            Self::String(s) => escape::quote_if_needed(s, escape::QuoteMode::SingleQuote),
            Self::Assignment(a) => {
                let mut s = a.name.to_string();
                let op = if a.append { "+=" } else { "=" };
                s.push_str(op);
                s.push_str(&escape::quote_if_needed(
                    a.value.to_string().as_str(),
                    escape::QuoteMode::SingleQuote,
                ));
                s.into()
            }
        }
    }
}

/// 命令执行时使用的目标 shell。
pub enum ShellForCommandTarget<'a> {
    /// 借用父 shell 作为目标 shell。
    Borrowed(&'a mut Shell),
    /// 使用独立持有的 shell 作为目标 shell。
    Owned(Box<Shell>),
}

/// 显式描述命令执行的目标 shell 及其父 shell 关系。
pub struct ShellForCommand<'a> {
    target: ShellForCommandTarget<'a>,
    parent: Option<&'a mut Shell>,
}

impl<'a> ShellForCommand<'a> {
    /// 使用父 shell 作为命令执行目标。
    pub const fn parent(shell: &'a mut Shell) -> Self {
        Self {
            target: ShellForCommandTarget::Borrowed(shell),
            parent: None,
        }
    }

    /// 使用子 shell 作为命令执行目标, 同时保留父 shell。
    pub const fn subshell(target: Box<Shell>, parent: &'a mut Shell) -> Self {
        Self {
            target: ShellForCommandTarget::Owned(target),
            parent: Some(parent),
        }
    }

    /// 返回命令目标是否是独立持有的 shell。
    pub const fn is_owned(&self) -> bool {
        match &self.target {
            ShellForCommandTarget::Borrowed(_) => false,
            ShellForCommandTarget::Owned(_) => true,
        }
    }

    /// 返回命令执行目标 shell。
    pub fn target(&self) -> &Shell {
        match &self.target {
            ShellForCommandTarget::Borrowed(shell) => shell,
            ShellForCommandTarget::Owned(target) => target,
        }
    }

    /// 返回命令执行目标 shell 的可变引用。
    pub fn target_mut(&mut self) -> &mut Shell {
        match &mut self.target {
            ShellForCommandTarget::Borrowed(shell) => shell,
            ShellForCommandTarget::Owned(target) => target,
        }
    }

    /// 消费 self, 返回 (owned target, parent shell)。
    /// Borrowed 情况下返回 (None, borrowed shell)。
    /// Owned 情况下返回 (Some(target), parent)。
    pub fn into_owned_and_parent(self) -> (Option<Box<Shell>>, &'a mut Shell) {
        match (self.target, self.parent) {
            (ShellForCommandTarget::Borrowed(shell), _) => (None, shell),
            (ShellForCommandTarget::Owned(target), Some(parent)) => (Some(target), parent),
            (ShellForCommandTarget::Owned(_), None) => {
                unreachable!("owned shell must have a parent shell")
            }
        }
    }
}

/// Composes a `std::process::Command` to execute the given command. Appropriately
/// configures the command name and arguments, redirections, injected file
/// descriptors, environment variables, etc.
///
/// # Arguments
///
/// * `context` - The execution context in which the command is being composed.
/// * `command_name` - The name of the command to execute.
/// * `argv0` - The value to use for `argv[0]` (may be different from the command).
/// * `args` - The arguments to pass to the command.
/// * `empty_env` - If true, the command will be executed with an empty environment; if false, the
///   command will inherit environment variables marked as exported in the provided `Shell`.
pub fn compose_std_command<S: AsRef<OsStr>>(
    context: &ExecutionContext<'_>,
    command_name: &str,
    argv0: &str,
    args: &[S],
    empty_env: bool,
) -> Result<std::process::Command, error::Error> {
    let mut cmd = std::process::Command::new(command_name);

    // Keep the call centralized even though this runtime currently treats argv[0]
    // override as a no-op.
    cmd.arg0(argv0);

    // Pass through args.
    cmd.args(args);

    // Use the shell's current working dir.
    cmd.current_dir(context.shell.working_dir());

    // Start with a clear environment.
    cmd.env_clear();

    // Add in exported variables.
    if !empty_env {
        for (k, v) in context.shell.env().iter_exported() {
            // NOTE: To match bash behavior, we only include exported variables
            // that are set (i.e., have a value). This means a variable that
            // shows up in `declare -p` but has no *set* value will be omitted.
            if v.value().is_set() {
                cmd.env(k.as_str(), v.value().to_cow_str(context.shell).as_ref());
            }
        }
        // Set _ to the resolved command path for external commands.
        cmd.env("_", command_name);
    }

    // Add in exported functions.
    if !empty_env {
        for (func_name, registration) in context.shell.funcs().iter() {
            if registration.is_exported() {
                let var_name = std::format!("BASH_FUNC_{func_name}%%");
                let value = std::format!("() {}", registration.definition().body);
                cmd.env(var_name, value);
            }
        }
    }

    // Redirect stdin, if applicable.
    match context.try_fd(OpenFiles::STDIN_FD) {
        Some(OpenFile::Stdin(_)) | None => (),
        Some(stdin_file) => {
            let as_stdio: Stdio = stdin_file.into();
            cmd.stdin(as_stdio);
        }
    }

    // Redirect stdout, if applicable.
    match context.try_fd(OpenFiles::STDOUT_FD) {
        Some(OpenFile::Stdout(_)) | None => (),
        Some(stdout_file) => {
            let as_stdio: Stdio = stdout_file.into();
            cmd.stdout(as_stdio);
        }
    }

    // Redirect stderr, if applicable.
    match context.try_fd(OpenFiles::STDERR_FD) {
        Some(OpenFile::Stderr(_)) | None => {}
        Some(stderr_file) => {
            let as_stdio: Stdio = stderr_file.into();
            cmd.stderr(as_stdio);
        }
    }

    // Inject any other fds.
    let other_files = context.iter_fds().filter(|(fd, _)| {
        *fd != OpenFiles::STDIN_FD && *fd != OpenFiles::STDOUT_FD && *fd != OpenFiles::STDERR_FD
    });
    cmd.inject_fds(other_files)?;

    Ok(cmd)
}

pub(crate) async fn on_preexecute(
    cmd: &mut commands::SimpleCommand<'_>,
) -> Result<(), error::Error> {
    // Set BASH_COMMAND before invoking the DEBUG trap (and generally before
    // executing commands).
    let full_cmd = cmd.args.iter().map(|arg| arg.to_string()).join(" ");
    cmd.shell.target_mut().env_mut().update_or_add(
        "BASH_COMMAND",
        variables::ShellValueLiteral::Scalar(full_cmd),
        |_| Ok(()),
        env::EnvironmentLookup::Anywhere,
        env::EnvironmentScope::Global,
    )?;

    // Fire the DEBUG trap if one is registered.
    if cmd.shell.target().traps().handles(traps::TrapSignal::Debug) {
        let _ = cmd
            .shell
            .target_mut()
            .invoke_trap_handler(traps::TrapSignal::Debug, &cmd.params)
            .await?;
    }

    Ok(())
}

/// Represents a simple command to be executed.
pub struct SimpleCommand<'a> {
    /// The shell to run the command in.
    shell: ShellForCommand<'a>,

    /// The execution parameters for the command.
    pub params: ExecutionParameters,

    /// The name of the command to execute.
    pub command_name: String,

    /// The arguments to the command, including the command itself.
    pub args: Vec<CommandArg>,

    /// Whether to consider shell functions when looking up the command name.
    /// If true, shell functions will be checked; if false, they will be ignored.
    pub use_functions: bool,

    /// Optional list of directories to search for external commands. If left
    /// `None`, the default search logic will be used.
    pub path_dirs: Option<Vec<PathBuf>>,

    /// The process group ID to use for externally executed commands. This may be
    /// `None`, in which case the default behavior will be used.
    pub process_group_id: Option<i32>,

    /// Optional override for the `argv[0]` value presented to an externally
    /// spawned process. When `None`, `command_name` is used.
    pub argv0: Option<String>,

    /// Optionally provides a function that can run after execution occurs. Note
    /// that it is *not* invoked if the shell is discarded during the execution
    /// process.
    #[allow(clippy::type_complexity)]
    pub post_execute: Option<fn(&mut Shell) -> Result<(), error::Error>>,
}

impl<'a> SimpleCommand<'a> {
    /// Creates a new `SimpleCommand` instance.
    ///
    /// # Arguments
    ///
    /// * `shell` - The shell in which to execute the command.
    /// * `params` - The execution parameters for the command.
    /// * `command_name` - The name of the command to execute.
    /// * `args` - The arguments to the command, including the command itself.
    pub const fn new(
        shell: ShellForCommand<'a>,
        params: ExecutionParameters,
        command_name: String,
        args: Vec<CommandArg>,
    ) -> Self {
        Self {
            shell,
            params,
            command_name,
            args,
            use_functions: true,
            path_dirs: None,
            process_group_id: None,
            argv0: None,
            post_execute: None,
        }
    }

    /// Executes the simple command.
    ///
    /// The command may be a builtin, a shell function, or an externally
    /// executed command. This function's implementation is responsible for
    /// dispatching it appropriately according to the context provided.
    #[allow(
        clippy::missing_panics_doc,
        reason = "these unwrap calls should not panic"
    )]
    pub async fn execute(mut self) -> Result<ExecutionSpawnResult, error::Error> {
        // First see if it's the name of a builtin.
        let builtin = self
            .shell
            .target()
            .builtins()
            .get(&self.command_name)
            .cloned();

        // Assuming we weren't requested not to do so, check if it's the name of
        // a shell function.
        if self.use_functions
            && let Some(func_registration) = self
                .shell
                .target()
                .funcs()
                .get(self.command_name.as_str())
                .cloned()
        {
            return self.execute_via_function(func_registration).await;
        }

        // If we haven't yet resolved the command name and found a builtin that's not disabled,
        // then invoke it.
        if let Some(builtin) = builtin
            && !builtin.disabled
        {
            return self.execute_via_builtin(builtin).await;
        }

        // We still haven't found a command to invoke. We'll need to look for an external command.
        if !sys::fs::contains_path_separator(&self.command_name) {
            // All else failed; if we were given path directories to search, try to look through
            // them for a matching executable. Otherwise, use our default search logic.
            let path = if let Some(path_dirs) = &self.path_dirs {
                pathsearch::search_for_executable(path_dirs.iter(), self.command_name.as_str())
                    .next()
            } else {
                self.shell
                    .target_mut()
                    .find_first_executable_in_path_using_cache(&self.command_name)
            };

            if let Some(path) = path {
                self.execute_via_external(&path)
            } else {
                // Bash updates $_ even when the command is not found, so mirror
                // that here before reporting the error.
                let last_arg = Self::take_last_arg(&self.args);
                self.shell.target_mut().update_last_arg_variable(last_arg);

                if let Some(post_execute) = self.post_execute {
                    let _ = post_execute(self.shell.target_mut());
                }

                Err(ErrorKind::CommandNotFound(self.command_name).into())
            }
        } else {
            let command_name = PathBuf::from(self.command_name.clone());
            self.execute_via_external(command_name.as_path())
        }
    }

    /// Extracts the owned string representation of the last argument of a
    /// command, suitable for recording into `$_`.
    fn take_last_arg(args: &[CommandArg]) -> Option<String> {
        args.last().map(ToString::to_string)
    }

    async fn execute_via_builtin(
        self,
        builtin: builtins::Registration,
    ) -> Result<ExecutionSpawnResult, error::Error> {
        if self.shell.is_owned() {
            let Self {
                shell,
                params,
                command_name,
                args,
                ..
            } = self;
            let (owned_shell, _) = shell.into_owned_and_parent();
            let Some(target) = owned_shell else {
                unreachable!("owned shell expected")
            };

            Ok(Self::execute_via_builtin_in_owned_shell(
                *target,
                params,
                builtin,
                command_name,
                args,
            ))
        } else {
            self.execute_via_builtin_in_parent_shell(builtin).await
        }
    }

    fn execute_via_builtin_in_owned_shell(
        mut shell: Shell,
        params: ExecutionParameters,
        builtin: builtins::Registration,
        command_name: String,
        args: Vec<CommandArg>,
    ) -> ExecutionSpawnResult {
        let last_arg = Self::take_last_arg(&args);
        let join_handle = compio::runtime::spawn(async move {
            let cmd_context = ExecutionContext {
                shell: &mut shell,
                command_name,
                params,
            };

            let result = execute_builtin_command(&builtin, cmd_context, args).await;

            // Update $_ after command execution.
            shell.update_last_arg_variable(last_arg);

            result
        });

        ExecutionSpawnResult::StartedTask(join_handle)
    }

    async fn execute_via_builtin_in_parent_shell(
        self,
        builtin: builtins::Registration,
    ) -> Result<ExecutionSpawnResult, error::Error> {
        let Self {
            mut shell,
            params,
            command_name,
            args,
            post_execute,
            ..
        } = self;
        let last_arg = Self::take_last_arg(&args);

        let cmd_context = ExecutionContext {
            shell: shell.target_mut(),
            command_name,
            params,
        };

        let result = execute_builtin_command(&builtin, cmd_context, args).await;

        // Update $_ after command execution.
        shell.target_mut().update_last_arg_variable(last_arg);

        if let Some(post_execute) = post_execute {
            let _ = post_execute(shell.target_mut());
        }

        let result = result?;

        Ok(result.into())
    }

    async fn execute_via_function(
        self,
        func_registration: functions::Registration,
    ) -> Result<ExecutionSpawnResult, error::Error> {
        let Self {
            mut shell,
            params,
            command_name,
            args,
            post_execute,
            ..
        } = self;
        let last_arg = Self::take_last_arg(&args);

        let cmd_context = ExecutionContext {
            shell: shell.target_mut(),
            command_name,
            params,
        };

        // Strip the function name off args.
        let result = invoke_shell_function(func_registration, cmd_context, &args[1..]).await;

        // $_ is reset *after* the function body runs, to the last argument of
        // the invocation (or the function name itself if zero args). Any
        // mutations made inside the body are overwritten — this matches bash,
        // where the caller observes only the invocation's last argument.
        shell.target_mut().update_last_arg_variable(last_arg);

        if let Some(post_execute) = post_execute {
            let _ = post_execute(shell.target_mut());
        }

        result
    }

    fn execute_via_external(self, path: &Path) -> Result<ExecutionSpawnResult, error::Error> {
        let Self {
            mut shell,
            params,
            command_name,
            args,
            process_group_id,
            argv0,
            post_execute,
            ..
        } = self;
        let last_arg = Self::take_last_arg(&args);

        let cmd_context = ExecutionContext {
            shell: shell.target_mut(),
            command_name,
            params,
        };

        let resolved_path = path.to_string_lossy();
        let result = execute_external_command(
            cmd_context,
            resolved_path.as_ref(),
            process_group_id,
            argv0.as_deref(),
            &args[1..],
        );

        // Update $_ after command execution.
        shell.target_mut().update_last_arg_variable(last_arg);

        if let Some(post_execute) = post_execute {
            let _ = post_execute(shell.target_mut());
        }

        result
    }
}

pub(crate) fn execute_external_command(
    context: ExecutionContext<'_>,
    executable_path: &str,
    process_group_id: Option<i32>,
    argv0_override: Option<&str>,
    args: &[CommandArg],
) -> Result<ExecutionSpawnResult, error::Error> {
    // Filter out the args; we only want strings.
    let cmd_args = args
        .iter()
        .filter_map(|e| {
            if let CommandArg::String(s) = e {
                Some(s)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    // Before we lose ownership of the open files, figure out if stdin will be a terminal.
    let child_stdin_is_terminal = context
        .try_fd(openfiles::OpenFiles::STDIN_FD)
        .is_some_and(|f| f.is_terminal());

    // Figure out if we should be setting up a new process group.
    let new_pg = matches!(
        context.params.process_group_policy,
        ProcessGroupPolicy::NewProcessGroup
    );

    // Compose the std::process::Command that encapsulates what we want to launch.
    // argv[0] defaults to context.command_name (the user-facing name of the
    // command) unless the caller specified an explicit override.
    let argv0 = argv0_override.unwrap_or(context.command_name.as_str());
    let mut cmd = compose_std_command(
        &context,
        executable_path,
        argv0,
        cmd_args.as_slice(),
        false, /* empty environment? */
    )?;

    // Set up process group state.
    if new_pg {
        // Check if we'll be doing terminal control setup (which includes setsid)
        if child_stdin_is_terminal && context.shell.options().external_cmd_leads_session {
            // Don't set process_group(0) - setsid() in pre_exec will handle it
            cmd.lead_session();
        } else {
            // Normal case: create new process group in current session
            cmd.process_group(0);
            if child_stdin_is_terminal {
                cmd.take_foreground();
            }
        }
    } else {
        // We need to join an established process group.
        if let Some(pgid) = process_group_id {
            cmd.process_group(pgid);
        }
    }

    // When tracing is enabled, report.
    log::debug!(
        target: trace_categories::COMMANDS,
        "Spawning: cmd='{} {}'",
        cmd.get_program().to_string_lossy(),
        cmd.get_args()
            .map(|a| a.to_string_lossy().to_string())
            .join(" ")
    );

    match sys::process::spawn(cmd) {
        Ok(child) => {
            // Retrieve the pid.
            #[expect(clippy::cast_possible_wrap)]
            let pid = child.id().map(|id| id as i32);
            let mut actual_pgid = process_group_id;
            if let Some(pid) = &pid {
                if new_pg {
                    actual_pgid = Some(*pid);
                }
            } else {
                log::warn!("could not retrieve pid for child process");
            }

            Ok(ExecutionSpawnResult::StartedProcess(
                processes::ChildProcess::new(child, pid, actual_pgid),
            ))
        }
        Err(spawn_err) => {
            if context.shell.options().interactive && sys::terminal::supports_foreground_control() {
                sys::terminal::move_self_to_foreground()?;
            }

            if spawn_err.kind() == std::io::ErrorKind::NotFound {
                if !context.shell.working_dir().exists() {
                    Err(
                        error::ErrorKind::WorkingDirMissing(context.shell.working_dir().to_owned())
                            .into(),
                    )
                } else {
                    Err(error::ErrorKind::CommandNotFound(context.command_name).into())
                }
            } else {
                Err(
                    error::ErrorKind::FailedToExecuteCommand(context.command_name, spawn_err)
                        .into(),
                )
            }
        }
    }
}

async fn execute_builtin_command(
    builtin: &builtins::Registration,
    context: ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> Result<ExecutionResult, error::Error> {
    match (builtin.execute_func)(context, args).await {
        Ok(result) => Ok(result),
        Err(e) => {
            // Broken pipe errors should silently return the appropriate exit code
            if let Some(io_err) = e.as_io_error()
                && io_err.kind() == std::io::ErrorKind::BrokenPipe
            {
                return Ok(ExecutionResult::new(error::io_error_exit_code(io_err)));
            }

            Err(e)
        }
    }
}

pub(crate) async fn invoke_shell_function(
    function: functions::Registration,
    mut context: ExecutionContext<'_>,
    args: &[CommandArg],
) -> Result<ExecutionSpawnResult, error::Error> {
    let ast::FunctionBody(body, redirects) = &function.definition().body;

    // Apply any redirects specified at function definition-time.
    if let Some(redirects) = redirects {
        for redirect in &redirects.0 {
            interp::setup_redirect(context.shell, &mut context.params, redirect).await?;
        }
    }

    let positional_args = args.iter().map(|a| a.to_string());

    // Pass through open files.
    let params = context.params.clone();

    // Note that we're going deeper. Once we do this, we need to make sure we don't bail early
    // before "exiting" the function.
    context.shell.enter_function(
        context.command_name.as_str(),
        &function,
        positional_args,
        &context.params,
    )?;

    // Invoke the function.
    let result = body.execute(context.shell, &params).await;

    // Clean up parameters so any owned files are closed.
    drop(params);

    // We've come back out, reflect it.
    context.shell.leave_function()?;

    // Get the actual execution result from the body of the function.
    let mut result = result?;

    // Handle control-flow.
    match result.next_control_flow {
        ExecutionControlFlow::BreakLoop { .. } | ExecutionControlFlow::ContinueLoop { .. } => {
            return error::unimp("break or continue returned from function invocation");
        }
        ExecutionControlFlow::ReturnFromFunctionOrScript => {
            // It's now been handled.
            result.next_control_flow = ExecutionControlFlow::Normal;
        }
        _ => {}
    }

    Ok(result.into())
}

pub(crate) async fn invoke_command_in_subshell_and_get_output(
    shell: &mut Shell,
    params: &ExecutionParameters,
    s: String,
) -> Result<String, error::Error> {
    // Instantiate a subshell to run the command in.
    let mut subshell = shell.clone();

    // Command substitutions don't inherit errexit by default. Only inherit it when
    // command_subst_inherits_errexit is enabled, otherwise disable errexit in the subshell.
    if !shell.options().command_subst_inherits_errexit {
        subshell.options_mut().exit_on_nonzero_command_exit = false;
    }

    // Get our own set of parameters we can customize and use.
    let mut params = params.clone();
    params.process_group_policy = ProcessGroupPolicy::SameProcessGroup;

    // Set up pipe so we can read the output.
    let (reader, writer) = std::io::pipe()?;
    params.set_fd(OpenFiles::STDOUT_FD, writer.into());

    let mut async_reader = sys::async_pipe::AsyncPipeReader::new(reader)?;

    let cmd_join_handle = compio::runtime::spawn(run_substitution_command(subshell, params, s));

    let output_str = async_reader.read_to_string().await?;

    // Now observe the command's completion.
    let run_result = cmd_join_handle
        .await
        .map_err(|err| error::ErrorKind::ThreadingError(err.to_string()))?;
    let cmd_result = run_result?;

    // Store the status.
    shell.set_last_exit_status(cmd_result.exit_code);

    // Note: $_ is naturally isolated from the parent because we cloned the
    // shell to run the substitution.

    Ok(output_str)
}

async fn run_substitution_command(
    mut shell: Shell,
    mut params: ExecutionParameters,
    command: String,
) -> Result<ExecutionResult, error::Error> {
    // Parse the string into a whole shell program.
    let parse_result = shell.parse_string(command);

    // Check for a command that is only an input redirection ("< file").
    // If detected, emulate `cat file` to stdout and return immediately.
    // If we failed to parse, then we'll fall below and handle it there.
    if let Ok(program) = &parse_result
        && let Some(redir) = try_unwrap_bare_input_redir_program(program)
    {
        interp::setup_redirect(&mut shell, &mut params, redir).await?;
        std::io::copy(&mut params.stdin(&shell), &mut params.stdout(&shell))?;
        return Ok(ExecutionResult::new(0));
    }

    // TODO(source-info): review this
    let source_info = crate::engine::SourceInfo::from("main");

    // Handle the parse result using default shell behavior.
    shell
        .run_parsed_result(parse_result, &source_info, &params)
        .await
}

// Detects a subshell command that consists solely of a single input redirection
// (e.g., "< file"), returning the IoRedirect when present.
fn try_unwrap_bare_input_redir_program(program: &ast::Program) -> Option<&ast::IoRedirect> {
    // We're looking for exactly one complete command...
    let [complete] = program.complete_commands.as_slice() else {
        return None;
    };

    // ...a single list item...
    let ast::CompoundList(items) = complete;
    let [item] = items.as_slice() else {
        return None;
    };

    // ...with a single pipeline (no && or || chaining)...
    let and_or = &item.0;
    if !and_or.additional.is_empty() {
        return None;
    }

    // ...not negated...
    let pipeline = &and_or.first;
    if pipeline.bang {
        return None;
    }

    // ...with a single command in the pipeline...
    let [ast::Command::Simple(simple_cmd)] = pipeline.seq.as_slice() else {
        return None;
    };

    // ...with no program word/name and no suffix...
    if simple_cmd.word_or_name.is_some() || simple_cmd.suffix.is_some() {
        return None;
    }

    // ...and exactly one prefix containing an I/O redirect...
    let prefix = simple_cmd.prefix.as_ref()?;
    let [ast::CommandPrefixOrSuffixItem::IoRedirect(redir)] = prefix.0.as_slice() else {
        return None;
    };

    // ...that is a file input redirection to a filename, targeting stdin.
    match redir {
        ast::IoRedirect::File(
            fd,
            ast::IoFileRedirectKind::Read,
            ast::IoFileRedirectTarget::Filename(..),
        ) if fd.is_none_or(|fd| fd == openfiles::OpenFiles::STDIN_FD) => Some(redir),
        _ => None,
    }
}
