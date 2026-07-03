//! Module defining the builder for creating shell instances.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

pub use shell_builder::State as ShellBuilderState;

use super::Shell;
use crate::engine::{
    ProfileLoadBehavior, RcLoadBehavior, ShellFd, ShellVariable, builtins, callstack, completion,
    env, error, extensions, functions, jobs, openfiles, options, pathcache,
    shell::KeyBindingsHelper, traps,
};

impl<S: shell_builder::IsComplete> ShellBuilder<S> {
    /// Returns a new shell instance created with the options provided. Runs any
    /// configuration loading as well.
    pub async fn build(self) -> Result<Shell, error::Error> {
        let mut options = self.build_settings();

        let profile = std::mem::take(&mut options.profile);
        let rc = std::mem::take(&mut options.rc);

        // Construct the shell.
        let mut shell = Shell::new(options)?;

        // Load profiles/configuration, unless skipped.
        if !profile.skip() || !rc.skip() {
            shell.load_config(&profile, &rc).await?;
        }

        Ok(shell)
    }
}

/// 为 ShellBuilder 生成一组 enable/disable 方法（单个与批量各一对）。
macro_rules! define_option_methods {
    (
        $enable_fn:ident, $enable_many_fn:ident, $enable_field:ident,
        $disable_fn:ident, $disable_many_fn:ident, $disable_field:ident
    ) => {
        /// 启用单个选项
        pub fn $enable_fn(mut self, option: impl Into<String>) -> Self {
            self.$enable_field.push(option.into());
            self
        }
        /// 启用多个选项
        pub fn $enable_many_fn(mut self, options: impl IntoIterator<Item: Into<String>>) -> Self {
            self.$enable_field
                .extend(options.into_iter().map(Into::into));
            self
        }
        /// 禁用单个选项
        pub fn $disable_fn(mut self, option: impl Into<String>) -> Self {
            self.$disable_field.push(option.into());
            self
        }
        /// 禁用多个选项
        pub fn $disable_many_fn(mut self, options: impl IntoIterator<Item: Into<String>>) -> Self {
            self.$disable_field
                .extend(options.into_iter().map(Into::into));
            self
        }
    };
}

impl<S: shell_builder::State> ShellBuilder<S> {
    define_option_methods!(
        enable_option,
        enable_options,
        enabled_options,
        disable_option,
        disable_options,
        disabled_options
    );
    define_option_methods!(
        enable_shopt_option,
        enable_shopt_options,
        enabled_shopt_options,
        disable_shopt_option,
        disable_shopt_options,
        disabled_shopt_options
    );

    /// Add a single builtin registration
    pub fn builtin(mut self, name: impl Into<String>, reg: builtins::Registration) -> Self {
        self.builtins.insert(name.into(), reg);
        self
    }

    /// Add many builtin registrations
    pub fn builtins(
        mut self,
        builtins: impl IntoIterator<Item = (String, builtins::Registration)>,
    ) -> Self {
        self.builtins.extend(builtins);
        self
    }

    /// Adds a single variable to be initialized in the shell.
    pub fn var(mut self, name: impl Into<String>, variable: ShellVariable) -> Self {
        self.vars.insert(name.into(), variable);
        self
    }
}

/// Options for creating a new shell.
#[derive(Default, bon::Builder)]
#[builder(
    builder_type(
        name = ShellBuilder,
        vis = "pub",
        doc {
        /// Builder for [Shell]
    }),
    finish_fn(
        name = build_settings,
        vis = "pub(self)",
    ),
    start_fn(
        vis = "pub(self)"
    )
)]
pub struct CreateOptions {
    /// Disabled options.
    #[builder(field)]
    pub disabled_options: Vec<String>,
    /// Enabled options.
    #[builder(field)]
    pub enabled_options: Vec<String>,
    /// Disabled shopt options.
    #[builder(field)]
    pub disabled_shopt_options: Vec<String>,
    /// Enabled shopt options.
    #[builder(field)]
    pub enabled_shopt_options: Vec<String>,
    /// Registered builtins.
    #[builder(field)]
    pub builtins: HashMap<String, builtins::Registration>,
    /// Provides a set of variables to be initialized in the shell. If present, they
    /// are assigned *after* inherited or well-known variables are set (when applicable).
    #[builder(field)]
    pub vars: HashMap<String, ShellVariable>,
    /// 错误格式化器实现.
    pub error_formatter: Option<Arc<dyn extensions::ErrorFormatter>>,
    /// Disallow overwriting regular files via output redirection.
    #[builder(default)]
    pub disallow_overwriting_regular_files_via_output_redirection: bool,
    /// Do not execute commands.
    #[builder(default)]
    pub do_not_execute_commands: bool,
    /// Exit after one command.
    #[builder(default)]
    pub exit_after_one_command: bool,
    /// Whether the shell is interactive.
    #[builder(default)]
    pub interactive: bool,
    /// Whether the shell is a login shell.
    #[builder(default)]
    pub login: bool,
    /// Whether to skip using a readline-like interface for input.
    #[builder(default)]
    pub no_editing: bool,
    /// System profile loading behavior.
    #[builder(default)]
    pub profile: ProfileLoadBehavior,
    /// Rc file loading behavior.
    #[builder(default)]
    pub rc: RcLoadBehavior,
    /// Whether to skip inheriting environment variables from the calling process.
    #[builder(default)]
    pub do_not_inherit_env: bool,
    /// Whether to skip initializing well-known variables.
    #[builder(default)]
    pub skip_well_known_vars: bool,
    /// Provides a set of initial open files to be tracked by the shell.
    #[builder(default)]
    pub fds: HashMap<ShellFd, openfiles::OpenFile>,
    /// Whether to launch external commands as session leaders.
    #[builder(default)]
    pub external_cmd_leads_session: bool,
    /// Initial working dir for the shell. If left unspecified, will be populated from
    /// the host environment.
    pub working_dir: Option<PathBuf>,
    /// Whether to print commands and arguments as they are read.
    #[builder(default)]
    pub print_commands_and_arguments: bool,
    /// Whether commands are being read from stdin.
    #[builder(default)]
    pub read_commands_from_stdin: bool,
    /// The name of the shell.
    pub shell_name: Option<String>,
    /// Base positional arguments for the shell (not including the shell name).
    pub shell_args: Option<Vec<String>>,
    /// Optionally provides a display string describing the version and variant of the shell.
    pub shell_product_display_str: Option<String>,
    /// Whether to treat expansion of unset variables as an error.
    #[builder(default)]
    pub treat_unset_variables_as_error: bool,
    /// Whether to enable error-on-exit behavior.
    #[builder(default)]
    pub exit_on_nonzero_command_exit: bool,
    /// Whether to disable pathname expansion.
    #[builder(default)]
    pub disable_pathname_expansion: bool,
    /// Whether to print verbose output.
    #[builder(default)]
    pub verbose: bool,
    /// Parser implementation to use.
    #[builder(default)]
    pub parser: crate::engine::parser::ParserImpl,
    /// Whether the shell is in command string mode (-c).
    #[builder(default)]
    pub command_string_mode: bool,
    /// Maximum function call depth.
    pub max_function_call_depth: Option<usize>,
    /// Key bindings helper for the shell to use.
    pub key_bindings: Option<KeyBindingsHelper>,
    /// Shell implementation version.
    pub shell_version: Option<String>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            error_formatter: Arc::new(extensions::DefaultErrorFormatter),
            traps: traps::TrapHandlerConfig::default(),
            open_files: openfiles::OpenFiles::default(),
            working_dir: PathBuf::default(),
            env: env::ShellEnvironment::default(),
            funcs: functions::FunctionEnv::default(),
            options: options::RuntimeOptions::default(),
            jobs: jobs::JobManager::default(),
            aliases: HashMap::default(),
            last_exit_status: 0,
            last_exit_status_change_count: 0,
            last_pipeline_statuses: vec![0],
            depth: 0,
            name: None,
            args: vec![],
            version: None,
            product_display_str: None,
            call_stack: callstack::CallStack::new(),
            directory_stack: vec![],
            completion_config: std::sync::Arc::new(completion::Config::default()),
            builtins: std::sync::Arc::new(HashMap::default()),
            program_location_cache: std::sync::Arc::new(pathcache::PathCache::default()),
            external_command_completion_cache: pathcache::ExecutableNameCache::default(),
            last_stopwatch_time: std::time::SystemTime::now(),
            last_stopwatch_offset: 0,
            parser_impl: crate::engine::parser::ParserImpl::default(),
            key_bindings: None,
            history: None,
        }
    }
}

impl Shell {
    /// Create an instance of [Shell] using the builder syntax
    pub fn builder() -> ShellBuilder<shell_builder::Empty> {
        CreateOptions::builder()
    }
}
