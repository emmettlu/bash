//! Defines shell options.

use crate::engine::options::RuntimeOptions;

type OptionGetter = fn(shell: &RuntimeOptions) -> bool;
type OptionSetter = fn(shell: &mut RuntimeOptions, value: bool) -> ();

macro_rules! opt {
    ($field:ident) => {
        ShellOptionDef::new(
            |options| options.$field,
            |options, value| options.$field = value,
        )
    };
}

/// Defines an option.
pub struct ShellOptionDef {
    /// Getter function that retrieves the current value of the option.
    getter: OptionGetter,
    /// Setter function that may be used to set the current value of the option.
    setter: OptionSetter,
}

impl ShellOptionDef {
    /// Constructs a new option definition.
    ///
    /// # Arguments
    ///
    /// * `getter` - A function that retrieves the current value of the option.
    /// * `setter` - A function that sets the current value of the option.
    const fn new(getter: OptionGetter, setter: OptionSetter) -> Self {
        Self { getter, setter }
    }

    /// Retrieves the current value of this option from the given runtime options.
    ///
    /// # Arguments
    ///
    /// * `options` - The runtime options to retrieve the value from.
    pub fn get(&self, options: &RuntimeOptions) -> bool {
        (self.getter)(options)
    }

    /// Sets the value of this option in the given runtime options.
    ///
    /// # Arguments
    ///
    /// * `options` - The runtime options to modify.
    /// * `value` - The new value to set for the option.
    pub fn set(&self, options: &mut RuntimeOptions, value: bool) {
        (self.setter)(options, value);
    }
}

/// Describes a shell option.
pub struct ShellOption {
    /// The name of the option.
    pub name: &'static str,
    /// The definition of the option.
    pub definition: &'static ShellOptionDef,
}

/// Describes a set of shell options.
pub struct ShellOptionSet {
    inner: &'static [(&'static str, ShellOptionDef)],
}

/// Kind of shell option.
#[derive(Clone, Copy)]
pub enum ShellOptionKind {
    /// `set` option.
    Set,
    /// `set -o` option.
    SetO,
    /// `shopt` option.
    Shopt,
}

/// Returns the options for the given shell option kind.
///
/// # Arguments
///
/// * `kind` - The kind of shell options to retrieve.
pub fn options(kind: ShellOptionKind) -> ShellOptionSet {
    match kind {
        ShellOptionKind::Set => ShellOptionSet { inner: SET_OPTIONS },
        ShellOptionKind::SetO => ShellOptionSet {
            inner: SET_O_OPTIONS,
        },
        ShellOptionKind::Shopt => ShellOptionSet {
            inner: SHOPT_OPTIONS,
        },
    }
}

impl ShellOptionSet {
    /// Returns an iterator over the options defined in this set.
    pub fn iter(&self) -> impl Iterator<Item = ShellOption> {
        self.inner
            .iter()
            .map(|(name, definition)| ShellOption { name, definition })
    }

    /// Returns the option with the given name, if it exists.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the option to retrieve.
    pub fn get(&self, name: &str) -> Option<&'static ShellOptionDef> {
        self.inner
            .iter()
            .find_map(|(option_name, definition)| (*option_name == name).then_some(definition))
    }
}

static SET_OPTIONS: &[(&str, ShellOptionDef)] = &[
    ("a", opt!(export_variables_on_modification)),
    ("b", opt!(notify_job_termination_immediately)),
    ("c", opt!(command_string_mode)),
    ("e", opt!(exit_on_nonzero_command_exit)),
    ("f", opt!(disable_filename_globbing)),
    ("h", opt!(remember_command_locations)),
    ("i", opt!(interactive)),
    ("k", opt!(place_all_assignment_args_in_command_env)),
    ("m", opt!(enable_job_control)),
    ("n", opt!(do_not_execute_commands)),
    ("p", opt!(real_effective_uid_mismatch)),
    ("t", opt!(exit_after_one_command)),
    ("u", opt!(treat_unset_variables_as_error)),
    ("v", opt!(print_shell_input_lines)),
    ("x", opt!(print_commands_and_arguments)),
    ("B", opt!(perform_brace_expansion)),
    (
        "C",
        opt!(disallow_overwriting_regular_files_via_output_redirection),
    ),
    ("E", opt!(shell_functions_inherit_err_trap)),
    ("H", opt!(enable_bang_style_history_substitution)),
    ("P", opt!(do_not_resolve_symlinks_when_changing_dir)),
    ("T", opt!(shell_functions_inherit_debug_and_return_traps)),
    ("s", opt!(read_commands_from_stdin)),
];

static SET_O_OPTIONS: &[(&str, ShellOptionDef)] = &[
    ("allexport", opt!(export_variables_on_modification)),
    ("braceexpand", opt!(perform_brace_expansion)),
    ("emacs", opt!(emacs_mode)),
    ("errexit", opt!(exit_on_nonzero_command_exit)),
    ("errtrace", opt!(shell_functions_inherit_err_trap)),
    (
        "functrace",
        opt!(shell_functions_inherit_debug_and_return_traps),
    ),
    ("hashall", opt!(remember_command_locations)),
    ("histexpand", opt!(enable_bang_style_history_substitution)),
    ("history", opt!(enable_command_history)),
    ("ignoreeof", opt!(ignore_eof)),
    ("interactive-comments", opt!(interactive_comments)),
    ("keyword", opt!(place_all_assignment_args_in_command_env)),
    ("monitor", opt!(enable_job_control)),
    (
        "noclobber",
        opt!(disallow_overwriting_regular_files_via_output_redirection),
    ),
    ("noexec", opt!(do_not_execute_commands)),
    ("noglob", opt!(disable_filename_globbing)),
    ("nolog", ShellOptionDef::new(|_| false, |_, _| ())),
    ("notify", opt!(notify_job_termination_immediately)),
    ("nounset", opt!(treat_unset_variables_as_error)),
    ("onecmd", opt!(exit_after_one_command)),
    ("physical", opt!(do_not_resolve_symlinks_when_changing_dir)),
    ("pipefail", opt!(return_last_failure_from_pipeline)),
    ("privileged", opt!(real_effective_uid_mismatch)),
    ("verbose", opt!(print_shell_input_lines)),
    ("vi", opt!(vi_mode)),
    ("xtrace", opt!(print_commands_and_arguments)),
];

static SHOPT_OPTIONS: &[(&str, ShellOptionDef)] = &[
    ("autocd", opt!(auto_cd)),
    ("array_expand_once", opt!(array_expand_once)),
    ("assoc_expand_once", opt!(assoc_expand_once)),
    ("bash_source_fullpath", opt!(bash_source_full_path)),
    ("cdable_vars", opt!(cdable_vars)),
    ("cdspell", opt!(cd_autocorrect_spelling)),
    ("checkhash", opt!(check_hashtable_before_command_exec)),
    ("checkjobs", opt!(check_jobs_before_exit)),
    (
        "checkwinsize",
        opt!(check_window_size_after_external_commands),
    ),
    ("cmdhist", opt!(save_multiline_cmds_in_history)),
    ("compat31", opt!(compat31)),
    ("compat32", opt!(compat32)),
    ("compat40", opt!(compat40)),
    ("compat41", opt!(compat41)),
    ("compat42", opt!(compat42)),
    ("compat43", opt!(compat43)),
    ("compat44", opt!(compat44)),
    (
        "complete_fullquote",
        opt!(quote_all_metachars_in_completion),
    ),
    ("direxpand", opt!(expand_dir_names_on_completion)),
    ("dirspell", opt!(autocorrect_dir_spelling_on_completion)),
    ("dotglob", opt!(glob_matches_dotfiles)),
    ("execfail", opt!(exit_on_exec_fail)),
    ("expand_aliases", opt!(expand_aliases)),
    ("extdebug", opt!(enable_debugger)),
    ("extglob", opt!(extended_globbing)),
    ("extquote", opt!(extquote)),
    ("failglob", opt!(fail_expansion_on_globs_without_match)),
    ("force_fignore", opt!(force_fignore)),
    ("globasciiranges", opt!(glob_ranges_use_c_locale)),
    ("globskipdots", opt!(glob_skip_dots)),
    ("globstar", opt!(enable_star_star_glob)),
    ("gnu_errfmt", opt!(errors_in_gnu_format)),
    ("histappend", opt!(append_to_history_file)),
    ("histreedit", opt!(allow_reedit_failed_history_subst)),
    ("histverify", opt!(allow_modifying_history_substitution)),
    ("hostcomplete", opt!(enable_hostname_completion)),
    ("huponexit", opt!(send_sighup_to_all_jobs_on_exit)),
    ("inherit_errexit", opt!(command_subst_inherits_errexit)),
    ("interactive_comments", opt!(interactive_comments)),
    ("lastpipe", opt!(run_last_pipeline_cmd_in_current_shell)),
    ("lithist", opt!(embed_newlines_in_multiline_cmds_in_history)),
    ("localvar_inherit", opt!(local_vars_inherit_value_and_attrs)),
    ("localvar_unset", opt!(localvar_unset)),
    ("login_shell", opt!(login_shell)),
    ("mailwarn", opt!(mail_warn)),
    ("no_empty_cmd_completion", opt!(no_empty_cmd_completion)),
    ("nocaseglob", opt!(case_insensitive_pathname_expansion)),
    ("nocasematch", opt!(case_insensitive_conditionals)),
    ("noexpand_translation", opt!(no_expand_translation)),
    ("nullglob", opt!(expand_non_matching_patterns_to_null)),
    ("patsub_replacement", opt!(patsub_replacement)),
    ("progcomp", opt!(programmable_completion)),
    ("progcomp_alias", opt!(programmable_completion_alias)),
    ("promptvars", opt!(expand_prompt_strings)),
    ("restricted_shell", opt!(restricted_shell)),
    ("shift_verbose", opt!(shift_verbose)),
    ("sourcepath", opt!(source_builtin_searches_path)),
    ("varredir_close", opt!(var_redir_close)),
    ("xpg_echo", opt!(echo_builtin_expands_escape_sequences)),
];
