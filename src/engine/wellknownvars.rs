use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::engine::variables::{self, DynamicShellValue, DynamicVariable};
use crate::engine::{Shell, ShellValue, ShellVariable, error, sys};

const BASH_MAJOR: u32 = 5;
const BASH_MINOR: u32 = 2;
const BASH_PATCH: u32 = 37;
const BASH_BUILD: u32 = 1;
const BASH_RELEASE: &str = "release";
const BASH_MACHINE: &str = "unknown";

const DEFAULT_LINENO: usize = 1;

/// Inherit environment variables from the host process into the shell's environment.
///
/// # Arguments
///
/// * `shell` - The shell instance to inherit environment variables into.
pub(crate) fn inherit_env_vars(shell: &mut Shell) -> Result<(), error::Error> {
    for (k, v) in sys::env::get_host_env_vars() {
        // See if it's a function exported by an ancestor process.
        if let Some(func_name) = k.strip_prefix("BASH_FUNC_")
            && let Some(func_name) = func_name.strip_suffix("%%")
        {
            // Intentionally best-effort; don't fail out of the shell if we can't
            // parse an incoming function.
            if shell.define_func_from_str(func_name, v.as_str()).is_ok()
                && let Some(func) = shell.func_mut(func_name)
            {
                func.export();
            }

            continue;
        }

        // Special case OLDPWD for bash compatibility.
        if k == "OLDPWD" {
            continue;
        }

        let mut var = ShellVariable::new(ShellValue::String(v));
        var.export();
        shell.env_mut().set_global(k, var)?;
    }

    Ok(())
}

#[expect(clippy::too_many_lines)]
pub(crate) fn init_well_known_vars(shell: &mut Shell) -> Result<(), error::Error> {
    // BASH
    if let Some(shell_name) = shell.current_shell_name().map(|s| s.to_string()) {
        shell
            .env_mut()
            .set_global("BASH", ShellVariable::new(shell_name.clone()))?;
        // Initialize $_ to the shell name ($0).
        shell.update_last_arg_variable(Some(shell_name));
    }

    // BASHOPTS
    let mut bashopts_var = ShellVariable::new(DynamicVariable::BashOpts);
    bashopts_var.set_readonly();
    shell.env_mut().set_global("BASHOPTS", bashopts_var)?;

    // BASHPID
    let mut bashpid_var = ShellVariable::new(ShellValue::String(std::process::id().to_string()));
    bashpid_var.treat_as_integer();
    shell.env_mut().set_global("BASHPID", bashpid_var)?;

    // BASH_ALIASES
    shell.env_mut().set_global(
        "BASH_ALIASES",
        ShellVariable::new(DynamicVariable::BashAliases),
    )?;

    // BASH_ARGC
    shell
        .env_mut()
        .set_global("BASH_ARGC", ShellVariable::new(DynamicVariable::BashArgc))?;

    // BASH_ARGV
    shell
        .env_mut()
        .set_global("BASH_ARGV", ShellVariable::new(DynamicVariable::BashArgv))?;

    // BASH_ARGV0
    shell
        .env_mut()
        .set_global("BASH_ARGV0", ShellVariable::new(DynamicVariable::BashArgv0))?;

    // TODO(vars): implement mutation of BASH_CMDS
    shell
        .env_mut()
        .set_global("BASH_CMDS", ShellVariable::new(DynamicVariable::BashCmds))?;

    // TODO(vars): implement BASH_COMMAND
    // TODO(vars): implement BASH_EXECUTION_STRING

    // BASH_LINENO
    shell.env_mut().set_global(
        "BASH_LINENO",
        ShellVariable::new(DynamicVariable::BashLineno),
    )?;

    // BASH_SOURCE
    shell.env_mut().set_global(
        "BASH_SOURCE",
        ShellVariable::new(DynamicVariable::BashSource),
    )?;

    // BASH_SUBSHELL
    shell.env_mut().set_global(
        "BASH_SUBSHELL",
        ShellVariable::new(DynamicVariable::BashSubshell),
    )?;

    // BASH_VERSINFO
    let mut bash_versinfo_var = ShellVariable::new(ShellValue::indexed_array_from_strs(
        [
            BASH_MAJOR.to_string().as_str(),
            BASH_MINOR.to_string().as_str(),
            BASH_PATCH.to_string().as_str(),
            BASH_BUILD.to_string().as_str(),
            BASH_RELEASE,
            BASH_MACHINE,
        ]
        .as_slice(),
    ));
    bash_versinfo_var.set_readonly();
    shell
        .env_mut()
        .set_global("BASH_VERSINFO", bash_versinfo_var)?;

    // BASH_VERSION
    // This is the Bash interface version.
    shell.env_mut().set_global(
        "BASH_VERSION",
        ShellVariable::new(std::format!(
            "{BASH_MAJOR}.{BASH_MINOR}.{BASH_PATCH}({BASH_BUILD})-{BASH_RELEASE}"
        )),
    )?;

    // COMP_WORDBREAKS
    let mut default_comp_wordbreaks = String::from(" \t\n\"\'><=;|&(:");
    if shell.options().enable_hostname_completion {
        default_comp_wordbreaks.push('@');
    }

    shell.env_mut().set_global(
        "COMP_WORDBREAKS",
        ShellVariable::new(default_comp_wordbreaks),
    )?;

    // DIRSTACK
    shell
        .env_mut()
        .set_global("DIRSTACK", ShellVariable::new(DynamicVariable::DirStack))?;

    // EPOCHREALTIME
    shell.env_mut().set_global(
        "EPOCHREALTIME",
        ShellVariable::new(DynamicVariable::EpochRealtime),
    )?;

    // EPOCHSECONDS
    shell.env_mut().set_global(
        "EPOCHSECONDS",
        ShellVariable::new(DynamicVariable::EpochSeconds),
    )?;

    // EUID
    if let Ok(euid) = sys::users::get_effective_uid() {
        let mut euid_var = ShellVariable::new(ShellValue::String(format!("{euid}")));
        euid_var.treat_as_integer().set_readonly();
        shell.env_mut().set_global("EUID", euid_var)?;
    }

    // FUNCNAME
    shell
        .env_mut()
        .set_global("FUNCNAME", ShellVariable::new(DynamicVariable::FuncName))?;

    // GROUPS
    // N.B. We could compute this up front, but we choose to make it dynamic so that we
    // don't have to make costly system calls if the user never accesses it.
    shell
        .env_mut()
        .set_global("GROUPS", ShellVariable::new(DynamicVariable::Groups))?;

    // HISTCMD
    let mut histcmd_var = ShellVariable::new(DynamicVariable::HistCmd);
    histcmd_var.treat_as_integer();
    shell.env_mut().set_global("HISTCMD", histcmd_var)?;

    // HISTFILE (if not already set)
    if !shell.env().is_set("HISTFILE")
        && let Some(home_dir) = shell.home_dir()
    {
        let histfile = home_dir.join(".bash_history");
        shell.env_mut().set_global(
            "HISTFILE",
            ShellVariable::new(ShellValue::String(histfile.to_string_lossy().to_string())),
        )?;
    }

    // HOSTNAME
    shell.env_mut().set_global(
        "HOSTNAME",
        ShellVariable::new(
            sys::network::get_hostname()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        ),
    )?;

    // HOSTTYPE
    shell.env_mut().set_global(
        "HOSTTYPE",
        ShellVariable::new(std::env::consts::ARCH.to_string()),
    )?;

    // IFS
    shell
        .env_mut()
        .set_global("IFS", ShellVariable::new(" \t\n"))?;

    // LINENO
    shell
        .env_mut()
        .set_global("LINENO", ShellVariable::new(DynamicVariable::LineNo))?;

    // MACHTYPE
    shell
        .env_mut()
        .set_global("MACHTYPE", ShellVariable::new(BASH_MACHINE))?;

    // OLDPWD (initialization)
    if !shell.env().is_set("OLDPWD") {
        let mut oldpwd_var =
            ShellVariable::new(ShellValue::Unset(variables::ShellValueUnsetType::Untyped));
        oldpwd_var.export();
        shell.env_mut().set_global("OLDPWD", oldpwd_var)?;
    }

    // OPTERR
    shell
        .env_mut()
        .set_global("OPTERR", ShellVariable::new("1"))?;

    // OPTIND
    let mut optind_var = ShellVariable::new("1");
    optind_var.treat_as_integer();
    shell.env_mut().set_global("OPTIND", optind_var)?;

    // OSTYPE
    // Match bash-compatible OSTYPE values for script compatibility. The binary
    // is Windows-only, but keeping the mapping centralized makes the intent clear
    // and avoids scattering string literals.
    let os_type = match std::env::consts::OS {
        "linux" => "linux-gnu",
        "android" => "linux-android",
        "macos" | "ios" | "tvos" | "watchos" | "visionos" => "darwin",
        "freebsd" => "freebsd",
        "netbsd" => "netbsd",
        "openbsd" => "openbsd",
        "dragonfly" => "dragonfly",
        "solaris" | "illumos" => "solaris",
        "windows" => "windows",
        _ => "unknown",
    };
    shell
        .env_mut()
        .set_global("OSTYPE", ShellVariable::new(os_type))?;

    // PATH (if not already set)
    if !shell.env().is_set("PATH") {
        let default_path_str = std::env::join_paths(sys::fs::get_default_executable_search_paths())
            .unwrap_or_else(|_| PathBuf::from("").into());
        shell
            .env_mut()
            .set_global("PATH", ShellVariable::new(default_path_str))?;
    }

    // PIPESTATUS
    // TODO(well-known-vars): Investigate what happens if this gets unset.
    // TODO(well-known-vars): Investigate if this needs to be saved/preserved across prompt display.
    shell.env_mut().set_global(
        "PIPESTATUS",
        ShellVariable::new(DynamicVariable::PipeStatus),
    )?;

    // PPID
    if let Some(ppid) = sys::terminal::get_parent_process_id() {
        let mut ppid_var = ShellVariable::new(ppid.to_string());
        ppid_var.treat_as_integer().set_readonly();
        shell.env_mut().set_global("PPID", ppid_var)?;
    }

    // RANDOM
    let mut random_var = ShellVariable::new(DynamicVariable::Random);
    random_var.treat_as_integer();
    shell.env_mut().set_global("RANDOM", random_var)?;

    // SECONDS
    shell
        .env_mut()
        .set_global("SECONDS", ShellVariable::new(DynamicVariable::Seconds))?;

    // SHELL (if not already set)
    if !shell.env().is_set("SHELL") {
        // Per docs, this should be the user's default login shell -- not the current shell.
        if let Some(default_shell) = sys::users::get_current_user_default_shell() {
            shell.env_mut().set_global(
                "SHELL",
                ShellVariable::new(default_shell.to_string_lossy().to_string()),
            )?;
        }
    }

    // SHELLOPTS
    let mut shellopts_var = ShellVariable::new(DynamicVariable::ShellOpts);
    shellopts_var.set_readonly();
    shell.env_mut().set_global("SHELLOPTS", shellopts_var)?;

    // SHLVL
    let input_shlvl = shell.env_str("SHLVL").unwrap_or_else(|| "0".into());
    let updated_shlvl = input_shlvl.as_ref().parse::<u32>().unwrap_or(0) + 1;
    let mut shlvl_var = ShellVariable::new(updated_shlvl.to_string());
    shlvl_var.export();
    shell.env_mut().set_global("SHLVL", shlvl_var)?;

    // SRANDOM
    let mut random_var = ShellVariable::new(DynamicVariable::SRandom);
    random_var.treat_as_integer();
    shell.env_mut().set_global("SRANDOM", random_var)?;

    // PS1 / PS2
    if shell.options().interactive {
        if !shell.env().is_set("PS1") {
            shell
                .env_mut()
                .set_global("PS1", ShellVariable::new(r"\s-\v\$ "))?;
        }

        if !shell.env().is_set("PS2") {
            shell
                .env_mut()
                .set_global("PS2", ShellVariable::new("> "))?;
        }
    }

    // PS4
    if !shell.env().is_set("PS4") {
        shell
            .env_mut()
            .set_global("PS4", ShellVariable::new("+ "))?;
    }

    //
    // PWD
    //
    // Reflect our actual working directory. There's a chance
    // we inherited an out-of-sync version of the variable. Future updates
    // will be handled by set_working_dir().
    //
    let pwd = shell.working_dir().to_string_lossy().to_string();
    let mut pwd_var = ShellVariable::new(pwd);
    pwd_var.export();
    shell.env_mut().set_global("PWD", pwd_var)?;

    // UID
    if let Ok(uid) = sys::users::get_current_uid() {
        let mut uid_var = ShellVariable::new(ShellValue::String(format!("{uid}")));
        uid_var.treat_as_integer().set_readonly();
        shell.env_mut().set_global("UID", uid_var)?;
    }

    Ok(())
}

impl DynamicShellValue {
    pub(crate) fn resolve(&self, shell: &Shell) -> ShellValue {
        match self.variable() {
            DynamicVariable::BashOpts => shell.options().shopt_optstr().into(),
            DynamicVariable::BashAliases => {
                let values = variables::ArrayLiteral(
                    shell
                        .aliases()
                        .iter()
                        .map(|(k, v)| (Some(k.to_owned()), v.to_owned()))
                        .collect::<Vec<_>>(),
                );

                ShellValue::associative_array_from_literals(values)
                    .unwrap_or_else(|_error| ShellValue::AssociativeArray(BTreeMap::new()))
            }
            DynamicVariable::BashArgc => get_bash_argc_value(shell),
            DynamicVariable::BashArgv => get_bash_argv_value(shell),
            DynamicVariable::BashArgv0 => {
                let argv0 = shell.current_shell_name().unwrap_or_default();
                argv0.to_string().into()
            }
            DynamicVariable::BashCmds => shell
                .program_location_cache()
                .to_value()
                .unwrap_or_else(|_error| ShellValue::AssociativeArray(BTreeMap::new())),
            DynamicVariable::BashLineno => get_bash_lineno_value(shell),
            DynamicVariable::BashSource => get_bash_source_value(shell),
            DynamicVariable::BashSubshell => shell.depth().to_string().into(),
            DynamicVariable::DirStack => shell
                .directory_stack()
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .into(),
            DynamicVariable::EpochRealtime => {
                let now = SystemTime::now();
                let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
                since_epoch.as_secs_f64().to_string().into()
            }
            DynamicVariable::EpochSeconds => {
                let now = SystemTime::now();
                let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
                since_epoch.as_secs().to_string().into()
            }
            DynamicVariable::FuncName => get_funcname_value(shell),
            DynamicVariable::Groups => {
                let groups = get_current_user_gids();
                ShellValue::indexed_array_from_strings(
                    groups.into_iter().map(|gid| gid.to_string()),
                )
            }
            DynamicVariable::HistCmd => shell
                .history()
                .map_or_else(|| "0".into(), |h| h.count().to_string().into()),
            DynamicVariable::LineNo => get_lineno(shell).to_string().into(),
            DynamicVariable::PipeStatus => ShellValue::indexed_array_from_strings(
                shell.last_pipeline_statuses().iter().map(|s| s.to_string()),
            ),
            DynamicVariable::Random => self.next_random_value().to_string().into(),
            DynamicVariable::Seconds => self.seconds_value().to_string().into(),
            DynamicVariable::ShellOpts => shell.options().seto_optstr().into(),
            DynamicVariable::SRandom => self.next_srandom_value().to_string().into(),
        }
    }
}

/// Returns a list of the current user's group IDs, with the effective GID at the front.
fn get_current_user_gids() -> Vec<u32> {
    let mut groups = sys::users::get_user_group_ids().unwrap_or_default();

    // If the effective GID is present but not in the first position in the list, then move
    // it there.
    if let Ok(gid) = sys::users::get_effective_gid()
        && let Some(index) = groups.iter().position(|&g| g == gid)
        && index > 0
    {
        // Move it to the front.
        groups.remove(index);
        groups.insert(0, gid);
    }

    groups
}

fn get_funcname_value(shell: &Shell) -> variables::ShellValue {
    let stack = shell.call_stack();

    if stack.iter_function_calls().next().is_none() {
        ShellValue::Unset(variables::ShellValueUnsetType::IndexedArray)
    } else {
        // When in a function, include both functions and sourced scripts in the stack
        stack
            .iter()
            .filter_map(|frame| match &frame.frame_type {
                crate::engine::callstack::FrameType::Function(func) => {
                    Some(func.function_name.as_str())
                }
                crate::engine::callstack::FrameType::Script(script) => {
                    // Only include sourced scripts, not run scripts
                    if matches!(
                        script.call_type,
                        crate::engine::callstack::ScriptCallType::Source
                    ) {
                        Some("source")
                    } else {
                        None
                    }
                }
                crate::engine::callstack::FrameType::TrapHandler(_)
                | crate::engine::callstack::FrameType::Eval
                | crate::engine::callstack::FrameType::CommandString
                | crate::engine::callstack::FrameType::InteractiveSession => None,
            })
            .collect::<Vec<_>>()
            .into()
    }
}

fn get_bash_lineno_value(shell: &Shell) -> variables::ShellValue {
    let stack = shell.call_stack();

    // BASH_LINENO[$i] contains the line number where FUNCNAME[$i] was called
    // This is extracted from the call_site of each frame
    if stack.iter_function_calls().next().is_none() {
        ShellValue::Unset(variables::ShellValueUnsetType::IndexedArray)
    } else {
        stack
            .iter()
            .enumerate()
            .filter_map(|(frame_idx, frame)| match &frame.frame_type {
                crate::engine::callstack::FrameType::Function(..)
                | crate::engine::callstack::FrameType::Script(..) => {
                    let caller_idx = frame_idx + 1;
                    if caller_idx < stack.depth() {
                        let caller_frame = &stack[caller_idx];
                        Some(
                            caller_frame
                                .current_line()
                                .unwrap_or(DEFAULT_LINENO)
                                .to_string(),
                        )
                    } else {
                        None
                    }
                }
                crate::engine::callstack::FrameType::TrapHandler(_)
                | crate::engine::callstack::FrameType::Eval
                | crate::engine::callstack::FrameType::CommandString
                | crate::engine::callstack::FrameType::InteractiveSession => None,
            })
            .collect::<Vec<_>>()
            .into()
    }
}

fn get_bash_source_value(shell: &Shell) -> variables::ShellValue {
    let stack = shell.call_stack();

    if stack.iter_function_calls().next().is_none() {
        let top_frame = stack.iter_script_calls().next();
        top_frame
            .map_or_else(Vec::new, |frame| vec![frame.source_info.source.clone()])
            .into()
    } else {
        // When in a function, include both functions and sourced scripts in the stack
        // This mirrors the FUNCNAME array structure
        stack
            .iter()
            .filter_map(|frame| match &frame.frame_type {
                crate::engine::callstack::FrameType::Function(func) => {
                    Some(func.function.source().source.clone())
                }
                crate::engine::callstack::FrameType::Script(script) => {
                    // Only include sourced scripts (matching the "source" in FUNCNAME)
                    if matches!(
                        script.call_type,
                        crate::engine::callstack::ScriptCallType::Source
                    ) {
                        Some(script.source_info.source.clone())
                    } else {
                        None
                    }
                }
                crate::engine::callstack::FrameType::TrapHandler(_)
                | crate::engine::callstack::FrameType::Eval => None,
                crate::engine::callstack::FrameType::CommandString
                | crate::engine::callstack::FrameType::InteractiveSession => None,
            })
            .collect::<Vec<_>>()
            .into()
    }
}

fn get_bash_argc_value(shell: &Shell) -> variables::ShellValue {
    if !shell.options().enable_debugger {
        return ShellValue::indexed_array_from_strs(&[]);
    }

    let stack = shell.call_stack();
    stack
        .iter()
        .filter_map(|frame| match &frame.frame_type {
            crate::engine::callstack::FrameType::Function(..)
            | crate::engine::callstack::FrameType::Script(..)
            | crate::engine::callstack::FrameType::CommandString
            | crate::engine::callstack::FrameType::InteractiveSession => {
                Some(frame.args.len().to_string())
            }
            crate::engine::callstack::FrameType::TrapHandler(_)
            | crate::engine::callstack::FrameType::Eval => None,
        })
        .collect::<Vec<_>>()
        .into()
}

fn get_bash_argv_value(shell: &Shell) -> variables::ShellValue {
    if !shell.options().enable_debugger {
        return ShellValue::indexed_array_from_strs(&[]);
    }

    let stack = shell.call_stack();
    let mut argv = Vec::new();

    for frame in stack.iter() {
        let include = match &frame.frame_type {
            crate::engine::callstack::FrameType::Function(..)
            | crate::engine::callstack::FrameType::Script(..)
            | crate::engine::callstack::FrameType::CommandString
            | crate::engine::callstack::FrameType::InteractiveSession => true,
            crate::engine::callstack::FrameType::TrapHandler(_)
            | crate::engine::callstack::FrameType::Eval => false,
        };

        if include {
            // Push args in reverse order per frame (last arg at lowest index = top of stack)
            for arg in frame.args.iter().rev() {
                argv.push(arg.clone());
            }
        }
    }

    argv.into()
}

fn get_lineno(shell: &Shell) -> usize {
    shell
        .call_stack()
        .current_frame()
        .and_then(|frame| frame.current_line())
        .unwrap_or(DEFAULT_LINENO)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::engine::variables::ShellValueLiteral;
    use anyhow::Result;

    fn assign_scalar(shell: &mut Shell, name: &str, value: &str) -> Result<()> {
        shell
            .env_mut()
            .get_mut(name)
            .unwrap()
            .1
            .assign(ShellValueLiteral::Scalar(value.to_owned()), false)?;
        Ok(())
    }

    fn scalar_value(shell: &Shell, name: &str) -> String {
        shell.env_str(name).unwrap().into_owned()
    }

    #[compio::test]
    async fn writable_dynamic_variables_apply_assignments() -> Result<()> {
        let mut shell = Shell::builder()
            .shell_name("original".to_owned())
            .build()
            .await?;

        assign_scalar(&mut shell, "RANDOM", "4321")?;
        let first = scalar_value(&shell, "RANDOM");
        let second = scalar_value(&shell, "RANDOM");
        assign_scalar(&mut shell, "RANDOM", "4321")?;
        assert_eq!(scalar_value(&shell, "RANDOM"), first);
        assert_eq!(scalar_value(&shell, "RANDOM"), second);

        assign_scalar(&mut shell, "SECONDS", "-3")?;
        let seconds = scalar_value(&shell, "SECONDS").parse::<i64>()?;
        assert!((-3..=0).contains(&seconds));

        shell
            .env_mut()
            .get_mut("BASH_ARGV0")
            .unwrap()
            .1
            .assign(ShellValueLiteral::Scalar("-suffix".to_owned()), true)?;
        assert_eq!(
            shell.current_shell_name().as_deref(),
            Some("original-suffix")
        );

        assign_scalar(&mut shell, "BASH_ARGV0", "renamed")?;
        assert_eq!(scalar_value(&shell, "BASH_ARGV0"), "renamed");
        assert_eq!(shell.current_shell_name().as_deref(), Some("renamed"));

        Ok(())
    }

    #[compio::test]
    async fn unsupported_and_readonly_dynamic_assignments_are_distinct() -> Result<()> {
        let mut shell = Shell::builder().build().await?;

        let unsupported = shell
            .env_mut()
            .get_mut("EPOCHSECONDS")
            .unwrap()
            .1
            .assign(ShellValueLiteral::Scalar("1".to_owned()), false)
            .unwrap_err();
        assert_eq!(
            unsupported.to_string(),
            "not yet implemented: assignment is unsupported for this dynamic variable"
        );

        let readonly = shell
            .env_mut()
            .get_mut("BASHOPTS")
            .unwrap()
            .1
            .assign(ShellValueLiteral::Scalar("1".to_owned()), false)
            .unwrap_err();
        assert!(matches!(
            readonly.kind(),
            error::ErrorKind::ReadonlyVariable
        ));

        Ok(())
    }
}
