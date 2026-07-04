//! Module defining the core shell structure and behavior.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::lock::Mutex;

use crate::engine::{
    ExecutionControlFlow, ExecutionResult, builtins, env::ShellEnvironment, error, extensions,
    functions, interfaces, jobs, keywords, openfiles, options::RuntimeOptions, pathcache,
    wellknownvars,
};

/// Type for storing a key bindings helper.
pub type KeyBindingsHelper = Arc<Mutex<dyn interfaces::KeyBindings>>;

/// Type alias for shell file descriptors.
pub type ShellFd = i32;

// NOTE: The submodule files below (e.g., `shell/traps.rs`, `shell/callstack.rs`) contain
// `impl Shell` blocks that provide methods coordinating with types defined in the
// corresponding top-level modules (e.g., `traps.rs`, `callstack.rs`). This is an intentional
// layered architecture: top-level modules define domain types and data structures, while
// shell/ submodules implement Shell methods that operate on those types.

mod builder;
mod builtin_registry;
mod callstack;
mod completion;
mod env;
mod execution;
mod expansion;
mod fs;
mod funcs;
mod history;
mod initscripts;
mod io;
mod job_control;
mod parsing;
mod prompts;
mod readline;
mod traps;

pub(crate) use builder::CreateOptions;
pub use builder::{ShellBuilder, ShellBuilderState};
pub use initscripts::{ProfileLoadBehavior, RcLoadBehavior};

/// Represents an instance of a shell.
///
pub struct Shell {
    /// 注入的错误格式化器.
    error_formatter: Arc<dyn extensions::ErrorFormatter>,

    /// Trap handler configuration for the shell.
    traps: crate::engine::traps::TrapHandlerConfig,

    /// Manages files opened and accessible via redirection operators.
    open_files: openfiles::OpenFiles,

    /// The current working directory.
    working_dir: PathBuf,

    /// The shell environment, containing shell variables.
    env: ShellEnvironment,

    /// Shell function definitions.
    funcs: functions::FunctionEnv,

    /// Runtime shell options.
    options: RuntimeOptions,

    /// State of managed jobs.
    jobs: jobs::JobManager,

    /// Shell aliases.
    aliases: HashMap<String, String>,

    /// The status of the last completed command.
    last_exit_status: u8,

    /// Tracks changes to `last_exit_status`.
    last_exit_status_change_count: usize,

    /// The status of each of the commands in the last pipeline.
    last_pipeline_statuses: Vec<u8>,

    /// Clone depth from the original ancestor shell.
    depth: usize,

    /// Shell name
    name: Option<String>,

    /// Positional shell arguments (not including shell name).
    args: Vec<String>,

    /// Shell version
    version: Option<String>,

    /// Detailed display string for the shell
    product_display_str: Option<String>,

    /// Function/script call stack.
    call_stack: crate::engine::callstack::CallStack,

    /// Directory stack used by pushd et al.
    directory_stack: Vec<PathBuf>,

    /// Completion configuration.
    completion_config: Arc<crate::engine::completion::Config>,

    /// Shell built-in commands.
    builtins: Arc<HashMap<String, builtins::Registration>>,

    /// Shell program location cache.
    program_location_cache: Arc<pathcache::PathCache>,

    /// Cached executable names used for interactive command completion.
    external_command_completion_cache: pathcache::ExecutableNameCache,

    /// Last "SECONDS" captured time.
    last_stopwatch_time: std::time::SystemTime,

    /// Last "SECONDS" offset requested.
    last_stopwatch_offset: u32,

    /// Parser implementation to use.
    parser_impl: crate::engine::parser::ParserImpl,

    /// Key bindings for the shell, optionally implemented by an interactive shell.
    key_bindings: Option<KeyBindingsHelper>,

    /// History of commands executed in the shell.
    history: Option<crate::engine::history::History>,
}

impl Clone for Shell {
    fn clone(&self) -> Self {
        Self {
            error_formatter: self.error_formatter.clone(),
            traps: self.traps.clone(),
            open_files: self.open_files.clone(),
            working_dir: self.working_dir.clone(),
            env: self.env.clone(),
            funcs: self.funcs.clone(),
            options: self.options.clone(),
            jobs: jobs::JobManager::new(),
            aliases: self.aliases.clone(),
            last_exit_status: self.last_exit_status,
            last_exit_status_change_count: self.last_exit_status_change_count,
            last_pipeline_statuses: self.last_pipeline_statuses.clone(),
            name: self.name.clone(),
            args: self.args.clone(),
            version: self.version.clone(),
            product_display_str: self.product_display_str.clone(),
            call_stack: {
                // Subshells must not inherit the parent's "currently handling signal X"
                // state; otherwise a trap handler that spawns a subshell would see itself
                // as already inside that handler and skip re-entrant delivery.
                let mut cs = self.call_stack.clone();
                cs.clear_active_trap_signals();
                cs
            },
            directory_stack: self.directory_stack.clone(),
            completion_config: self.completion_config.clone(),
            builtins: self.builtins.clone(),
            program_location_cache: self.program_location_cache.clone(),
            external_command_completion_cache: self.external_command_completion_cache.clone(),
            last_stopwatch_time: self.last_stopwatch_time,
            last_stopwatch_offset: self.last_stopwatch_offset,
            parser_impl: self.parser_impl,
            key_bindings: self.key_bindings.clone(),
            history: self.history.clone(),
            depth: self.depth + 1,
        }
    }
}

impl Shell {
    /// 创建一个用于子 shell 语义的 shell 副本。
    ///
    /// 这不是普通值复制: 子 shell 会继承大部分运行状态, 但会重置作业表,
    /// 清理正在处理的 trap 状态, 并递增 clone depth。调用点应优先使用此方法,
    /// 避免把 `Clone` 误认为无语义的简单复制。
    #[must_use]
    pub(crate) fn fork_subshell(&self) -> Self {
        self.clone()
    }
}

impl AsRef<Self> for Shell {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl AsMut<Self> for Shell {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

impl Shell {
    /// Returns a new shell instance created with the given options.
    /// Does *not* load any configuration files (e.g., bashrc).
    ///
    /// # Arguments
    ///
    /// * `options` - The options to use when creating the shell.
    pub(crate) fn new(options: CreateOptions) -> Result<Self, error::Error> {
        // Compute runtime options before moving fields out of `options`.
        let runtime_options = RuntimeOptions::defaults_from(&options);

        // Instantiate the shell with some defaults.
        let mut shell = Self {
            error_formatter: options
                .error_formatter
                .unwrap_or_else(|| Arc::new(extensions::DefaultErrorFormatter)),
            open_files: openfiles::OpenFiles::new(),
            options: runtime_options,
            name: options.shell_name,
            args: options.shell_args.unwrap_or_default(),
            version: options.shell_version,
            product_display_str: options.shell_product_display_str,
            working_dir: options.working_dir.map_or_else(std::env::current_dir, Ok)?,
            builtins: Arc::new(options.builtins),
            parser_impl: options.parser,
            key_bindings: options.key_bindings,
            ..Self::default()
        };

        // Add in any open files provided.
        shell.open_files.update_from(options.fds.into_iter());

        // TODO(patterns): Without this a script that sets extglob will fail because we
        // parse the entire script with the same settings.
        shell.options.extended_globbing = true;

        // If requested, seed parameters from environment.
        if !options.do_not_inherit_env {
            wellknownvars::inherit_env_vars(&mut shell)?;
        }

        // If requested, set well-known variables.
        if !options.skip_well_known_vars {
            wellknownvars::init_well_known_vars(&mut shell)?;
        }

        // Set any provided variables.
        for (var_name, var_value) in options.vars {
            shell.env.set_global(var_name, var_value)?;
        }

        // Set up history, if relevant. Do NOT fail if we can't load history.
        if shell.options.enable_command_history {
            shell.history = shell
                .load_history()
                .unwrap_or_default()
                .or_else(|| Some(crate::engine::history::History::default()));
        }

        Ok(shell)
    }
}

impl Shell {
    /// Increments the interactive line offset in the shell by the indicated number
    /// of lines.
    ///
    /// # Arguments
    ///
    /// * `delta` - The number of lines to increment the current line offset by.
    pub fn increment_interactive_line_offset(&mut self, delta: usize) {
        self.call_stack.increment_current_line_offset(delta);
    }

    /// Updates the currently executing command in the shell.
    pub fn set_current_cmd(&mut self, cmd: &impl crate::parser::ast::Node) {
        self.call_stack
            .set_current_pos(cmd.location().map(|span| span.start));
    }

    /// Updates the `$_` shell variable (last-argument of the previous simple
    /// command).
    ///
    /// Passes `Some(last_arg)` to record the last argument of the just-executed
    /// command, or `None` to clear `$_` (used for assignment-only statements,
    /// which bash treats as having no "last argument").
    ///
    /// The update is applied in-place so that attributes on `_` (notably
    /// `readonly`) are preserved: attempting to update a readonly `_` is a
    /// silent no-op, matching bash's observable stdout behavior.
    pub(crate) fn update_last_arg_variable(&mut self, last_arg: Option<String>) {
        // Bash refuses to update a readonly `_`, emitting an error to stderr
        // on each attempt. We silently skip the update here — the observable
        // stdout effect ($_ stays unchanged) matches bash; the missing stderr
        // diagnostics are harmless.
        if self
            .env
            .get_using_policy("_", crate::engine::env::EnvironmentLookup::Anywhere)
            .is_some_and(|v| v.is_readonly())
        {
            return;
        }

        // Replace the variable entirely (fresh, non-exported). This matches
        // bash, which never exports `_` — even under `set -a` — and always
        // clears any previously-set attributes (except readonly, handled
        // above).
        let value = last_arg.unwrap_or_default();
        let _ = self
            .env
            .set_global("_", crate::engine::variables::ShellVariable::new(value));
    }

    /// Applies errexit semantics to a result if enabled and appropriate.
    /// This should be called at "statement boundaries" where errexit should be checked.
    ///
    /// # Arguments
    ///
    /// * `result` - The execution result to potentially modify.
    pub const fn apply_errexit_if_enabled(&self, result: &mut ExecutionResult) {
        if self.options.exit_on_nonzero_command_exit
            && !result.is_success()
            && result.is_normal_flow()
        {
            result.next_control_flow = ExecutionControlFlow::ExitShell;
        }
    }

    /// Returns the keywords that are reserved by the shell.
    pub(crate) fn get_keywords(&self) -> Vec<&str> {
        keywords::KEYWORDS.iter().copied().collect()
    }

    /// Checks if the given string is a keyword reserved in this shell.
    ///
    /// # Arguments
    ///
    /// * `s` - The string to check.
    pub fn is_keyword(&self, s: &str) -> bool {
        keywords::KEYWORDS.contains(s)
    }

    pub(crate) const fn last_exit_status_change_count(&self) -> usize {
        self.last_exit_status_change_count
    }
}

/// Shell 状态访问方法.
impl Shell {
    /// 返回 shell 的调用栈.
    pub fn call_stack(&self) -> &crate::engine::callstack::CallStack {
        &self.call_stack
    }

    /// 返回 shell 的运行时选项.
    pub fn options(&self) -> &RuntimeOptions {
        &self.options
    }

    /// 返回当前 subshell 嵌套深度, 0 表示非 subshell.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// 返回 shell 的历史记录 (如果存在).
    pub fn history(&self) -> Option<&crate::engine::history::History> {
        self.history.as_ref()
    }

    /// 返回上一条管道中各命令的退出状态.
    pub fn last_pipeline_statuses(&self) -> &[u8] {
        &self.last_pipeline_statuses
    }

    /// 返回上次 SECONDS 计时的起始时刻.
    pub fn last_stopwatch_time(&self) -> std::time::SystemTime {
        self.last_stopwatch_time
    }

    /// 返回上次 SECONDS 的偏移量.
    pub fn last_stopwatch_offset(&self) -> u32 {
        self.last_stopwatch_offset
    }

    /// 返回当前 shell 名称 ($0), 受调用栈影响.
    pub fn current_shell_name(&self) -> Option<Cow<'_, str>> {
        for frame in self.call_stack.iter() {
            if frame.frame_type.is_run_script() {
                return Some(frame.frame_type.name());
            }
        }
        self.name.as_deref().map(|name| name.into())
    }

    /// 返回当前工作目录.
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// 返回 shell 的别名表.
    pub fn aliases(&self) -> &HashMap<String, String> {
        &self.aliases
    }

    /// 返回命令路径缓存.
    pub fn program_location_cache(&self) -> &pathcache::PathCache {
        self.program_location_cache.as_ref()
    }

    /// 返回目录栈.
    pub fn directory_stack(&self) -> &[PathBuf] {
        &self.directory_stack
    }
}

/// Shell 的其他状态访问方法.
impl Shell {
    /// 返回 shell 是否处于 subshell 环境.
    pub fn is_subshell(&self) -> bool {
        self.depth > 0
    }

    /// 返回 shell 环境 (变量存储).
    pub fn env(&self) -> &ShellEnvironment {
        &self.env
    }

    /// 返回 shell 环境的可变引用.
    pub fn env_mut(&mut self) -> &mut ShellEnvironment {
        &mut self.env
    }

    /// 返回运行时选项的可变引用.
    pub fn options_mut(&mut self) -> &mut RuntimeOptions {
        &mut self.options
    }

    /// 返回别名表的可变引用.
    pub fn aliases_mut(&mut self) -> &mut HashMap<String, String> {
        &mut self.aliases
    }

    /// 返回作业管理器.
    pub fn jobs(&self) -> &jobs::JobManager {
        &self.jobs
    }

    /// 返回作业管理器的可变引用.
    pub fn jobs_mut(&mut self) -> &mut jobs::JobManager {
        &mut self.jobs
    }

    /// 返回 trap 处理器配置.
    pub fn traps(&self) -> &crate::engine::traps::TrapHandlerConfig {
        &self.traps
    }

    /// 返回 trap 处理器配置的可变引用.
    pub fn traps_mut(&mut self) -> &mut crate::engine::traps::TrapHandlerConfig {
        &mut self.traps
    }

    /// 返回目录栈的可变引用.
    pub fn directory_stack_mut(&mut self) -> &mut Vec<PathBuf> {
        &mut self.directory_stack
    }

    /// 返回管道退出状态列表的可变引用.
    pub fn last_pipeline_statuses_mut(&mut self) -> &mut Vec<u8> {
        &mut self.last_pipeline_statuses
    }

    /// 返回命令路径缓存的可变引用.
    pub fn program_location_cache_mut(&mut self) -> &mut pathcache::PathCache {
        Arc::make_mut(&mut self.program_location_cache)
    }

    /// 返回补全配置.
    pub fn completion_config(&self) -> &crate::engine::completion::Config {
        self.completion_config.as_ref()
    }

    /// 返回补全配置的可变引用.
    pub fn completion_config_mut(&mut self) -> &mut crate::engine::completion::Config {
        Arc::make_mut(&mut self.completion_config)
    }

    /// 返回 shell 的打开文件集合.
    pub fn open_files(&self) -> &openfiles::OpenFiles {
        &self.open_files
    }

    /// 返回打开文件集合的可变引用.
    pub fn open_files_mut(&mut self) -> &mut openfiles::OpenFiles {
        &mut self.open_files
    }

    /// 返回历史记录的可变引用.
    pub fn history_mut(&mut self) -> Option<&mut crate::engine::history::History> {
        self.history.as_mut()
    }

    /// 返回 shell 的版本字符串.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// 返回上一条命令的退出状态.
    pub fn last_exit_status(&self) -> u8 {
        self.last_exit_status
    }

    /// 更新上一条命令的退出状态.
    pub fn set_last_exit_status(&mut self, status: u8) {
        self.last_exit_status = status;
        self.last_exit_status_change_count += 1;
    }

    /// 返回按键绑定辅助器.
    pub fn key_bindings(&self) -> Option<&KeyBindingsHelper> {
        self.key_bindings.as_ref()
    }

    /// 设置按键绑定辅助器.
    pub fn set_key_bindings(&mut self, key_bindings: Option<KeyBindingsHelper>) {
        self.key_bindings = key_bindings;
    }

    /// 返回当前工作目录的可变引用 (crate 内部使用).
    pub(crate) fn working_dir_mut(&mut self) -> &mut PathBuf {
        &mut self.working_dir
    }

    /// 返回产品显示名称.
    pub fn product_display_str(&self) -> Option<&str> {
        self.product_display_str.as_deref()
    }
}
