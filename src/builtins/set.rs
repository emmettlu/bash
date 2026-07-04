use std::collections::HashMap;
use std::io::Write;

use itertools::Itertools;

use super::common::PlusMinusFlag;
use crate::engine::{ExecutionResult, builtins, variables};

#[derive(Default)]
pub(crate) struct SetOption {
    enable: Option<Vec<String>>,
    disable: Option<Vec<String>>,
}

/// Manage set-based shell options.
#[derive(Default)]
pub(crate) struct SetCommand {
    export_variables_on_modification: PlusMinusFlag,
    notify_job_termination_immediately: PlusMinusFlag,
    exit_on_nonzero_command_exit: PlusMinusFlag,
    disable_filename_globbing: PlusMinusFlag,
    remember_command_locations: PlusMinusFlag,
    place_all_assignment_args_in_command_env: PlusMinusFlag,
    enable_job_control: PlusMinusFlag,
    do_not_execute_commands: PlusMinusFlag,
    real_effective_uid_mismatch: PlusMinusFlag,
    exit_after_one_command: PlusMinusFlag,
    treat_unset_variables_as_error: PlusMinusFlag,
    print_shell_input_lines: PlusMinusFlag,
    print_commands_and_arguments: PlusMinusFlag,
    perform_brace_expansion: PlusMinusFlag,
    disallow_overwriting_regular_files_via_output_redirection: PlusMinusFlag,
    shell_functions_inherit_err_trap: PlusMinusFlag,
    enable_bang_style_history_substitution: PlusMinusFlag,
    do_not_resolve_symlinks_when_changing_dir: PlusMinusFlag,
    shell_functions_inherit_debug_and_return_traps: PlusMinusFlag,

    set_option: SetOption,

    positional_args: Vec<String>,
}

fn parse_set_args<I>(args: I) -> Result<SetCommand, String>
where
    I: IntoIterator<Item = String>,
{
    let mut command = SetCommand::default();
    let mut args = builtins::BuiltinArgs::new(args);

    while let Some(arg) = args.next_arg() {
        if arg == "-" || arg == "--" {
            command.positional_args.push(arg);
            command.positional_args.extend(args.rest());
            break;
        }

        let Some((enabled, flags)) = parse_set_prefix(&arg) else {
            command.positional_args.push(arg);
            command.positional_args.extend(args.rest());
            break;
        };

        if flags.is_empty() {
            command.positional_args.push(arg);
            command.positional_args.extend(args.rest());
            break;
        }

        parse_set_flags(&mut args, &mut command, flags, enabled)?;
    }

    Ok(command)
}

fn parse_set_prefix(arg: &str) -> Option<(bool, &str)> {
    if let Some(flags) = arg.strip_prefix('-') {
        Some((true, flags))
    } else {
        arg.strip_prefix('+').map(|flags| (false, flags))
    }
}

fn parse_set_flags(
    args: &mut builtins::BuiltinArgs,
    command: &mut SetCommand,
    flags: &str,
    enabled: bool,
) -> Result<(), String> {
    for (idx, flag) in flags.char_indices() {
        let rest_start = idx + flag.len_utf8();
        if flag == 'o' {
            let value = optional_set_o_value(args, flags, rest_start);
            let target = if enabled {
                &mut command.set_option.enable
            } else {
                &mut command.set_option.disable
            };
            add_set_o_value(target, value);
            break;
        }

        let Some(target) = command.flag_mut(flag) else {
            let prefix = if enabled { '-' } else { '+' };
            return Err(format!("set: {prefix}{flag}: invalid option"));
        };
        target.set(enabled);
    }
    Ok(())
}

fn optional_set_o_value(
    args: &mut builtins::BuiltinArgs,
    flags: &str,
    rest_start: usize,
) -> Option<String> {
    if rest_start < flags.len() {
        Some(flags[rest_start..].to_owned())
    } else {
        args.next_arg()
    }
}

fn add_set_o_value(target: &mut Option<Vec<String>>, value: Option<String>) {
    match (target, value) {
        (slot @ None, Some(value)) => *slot = Some(vec![value]),
        (Some(values), Some(value)) => values.push(value),
        (slot @ None, None) => *slot = Some(Vec::new()),
        (Some(_), None) => {}
    }
}

impl SetCommand {
    fn flag_mut(&mut self, flag: char) -> Option<&mut PlusMinusFlag> {
        match flag {
            'a' => Some(&mut self.export_variables_on_modification),
            'b' => Some(&mut self.notify_job_termination_immediately),
            'e' => Some(&mut self.exit_on_nonzero_command_exit),
            'f' => Some(&mut self.disable_filename_globbing),
            'h' => Some(&mut self.remember_command_locations),
            'k' => Some(&mut self.place_all_assignment_args_in_command_env),
            'm' => Some(&mut self.enable_job_control),
            'n' => Some(&mut self.do_not_execute_commands),
            'p' => Some(&mut self.real_effective_uid_mismatch),
            't' => Some(&mut self.exit_after_one_command),
            'u' => Some(&mut self.treat_unset_variables_as_error),
            'v' => Some(&mut self.print_shell_input_lines),
            'x' => Some(&mut self.print_commands_and_arguments),
            'B' => Some(&mut self.perform_brace_expansion),
            'C' => Some(&mut self.disallow_overwriting_regular_files_via_output_redirection),
            'E' => Some(&mut self.shell_functions_inherit_err_trap),
            'H' => Some(&mut self.enable_bang_style_history_substitution),
            'P' => Some(&mut self.do_not_resolve_symlinks_when_changing_dir),
            'T' => Some(&mut self.shell_functions_inherit_debug_and_return_traps),
            _ => None,
        }
    }
}

impl builtins::Command for SetCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        parse_set_args(args)
    }

    #[expect(clippy::too_many_lines)]
    #[allow(clippy::useless_let_if_seq)]
    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        let mut result = ExecutionResult::success();

        let mut saw_option = false;

        if let Some(value) = self.print_commands_and_arguments.to_bool() {
            context.shell.options_mut().print_commands_and_arguments = value;
            saw_option = true;
        }

        if let Some(value) = self.export_variables_on_modification.to_bool() {
            context.shell.options_mut().export_variables_on_modification = value;
            saw_option = true;
        }

        if let Some(value) = self.notify_job_termination_immediately.to_bool() {
            context
                .shell
                .options_mut()
                .notify_job_termination_immediately = value;
            saw_option = true;
        }

        if let Some(value) = self.exit_on_nonzero_command_exit.to_bool() {
            context.shell.options_mut().exit_on_nonzero_command_exit = value;
            saw_option = true;
        }

        if let Some(value) = self.disable_filename_globbing.to_bool() {
            context.shell.options_mut().disable_filename_globbing = value;
            saw_option = true;
        }

        if let Some(value) = self.remember_command_locations.to_bool() {
            context.shell.options_mut().remember_command_locations = value;
            saw_option = true;
        }

        if let Some(value) = self.place_all_assignment_args_in_command_env.to_bool() {
            context
                .shell
                .options_mut()
                .place_all_assignment_args_in_command_env = value;
            saw_option = true;
        }

        if let Some(value) = self.enable_job_control.to_bool() {
            context.shell.options_mut().enable_job_control = value;
            saw_option = true;
        }

        if let Some(value) = self.do_not_execute_commands.to_bool() {
            context.shell.options_mut().do_not_execute_commands = value;
            saw_option = true;
        }

        if let Some(value) = self.real_effective_uid_mismatch.to_bool() {
            context.shell.options_mut().real_effective_uid_mismatch = value;
            saw_option = true;
        }

        if let Some(value) = self.exit_after_one_command.to_bool() {
            context.shell.options_mut().exit_after_one_command = value;
            saw_option = true;
        }

        if let Some(value) = self.treat_unset_variables_as_error.to_bool() {
            context.shell.options_mut().treat_unset_variables_as_error = value;
            saw_option = true;
        }

        if let Some(value) = self.print_shell_input_lines.to_bool() {
            context.shell.options_mut().print_shell_input_lines = value;
            saw_option = true;
        }

        if let Some(value) = self.print_commands_and_arguments.to_bool() {
            context.shell.options_mut().print_commands_and_arguments = value;
            saw_option = true;
        }

        if let Some(value) = self.perform_brace_expansion.to_bool() {
            context.shell.options_mut().perform_brace_expansion = value;
            saw_option = true;
        }

        if let Some(value) = self
            .disallow_overwriting_regular_files_via_output_redirection
            .to_bool()
        {
            context
                .shell
                .options_mut()
                .disallow_overwriting_regular_files_via_output_redirection = value;
            saw_option = true;
        }

        if let Some(value) = self.shell_functions_inherit_err_trap.to_bool() {
            context.shell.options_mut().shell_functions_inherit_err_trap = value;
            saw_option = true;
        }

        if let Some(value) = self.enable_bang_style_history_substitution.to_bool() {
            context
                .shell
                .options_mut()
                .enable_bang_style_history_substitution = value;
            saw_option = true;
        }

        if let Some(value) = self.do_not_resolve_symlinks_when_changing_dir.to_bool() {
            context
                .shell
                .options_mut()
                .do_not_resolve_symlinks_when_changing_dir = value;
            saw_option = true;
        }

        if let Some(value) = self
            .shell_functions_inherit_debug_and_return_traps
            .to_bool()
        {
            context
                .shell
                .options_mut()
                .shell_functions_inherit_debug_and_return_traps = value;
            saw_option = true;
        }

        let mut named_options: HashMap<String, bool> = HashMap::new();
        if let Some(option_names) = &self.set_option.disable {
            saw_option = true;
            if option_names.is_empty() {
                for option in crate::engine::namedoptions::options(
                    crate::engine::namedoptions::ShellOptionKind::SetO,
                )
                .iter()
                .sorted_by_key(|option| option.name)
                {
                    let option_value = option.definition.get(context.shell.options());
                    let option_value_str = if option_value { "-o" } else { "+o" };
                    writeln!(context.stdout(), "set {option_value_str} {}", option.name)?;
                }
            } else {
                for option_name in option_names {
                    named_options.insert(option_name.to_owned(), false);
                }
            }
        }
        if let Some(option_names) = &self.set_option.enable {
            saw_option = true;
            if option_names.is_empty() {
                for option in crate::engine::namedoptions::options(
                    crate::engine::namedoptions::ShellOptionKind::SetO,
                )
                .iter()
                .sorted_by_key(|option| option.name)
                {
                    let option_value = option.definition.get(context.shell.options());
                    let option_value_str = if option_value { "on" } else { "off" };
                    writeln!(context.stdout(), "{:15}\t{option_value_str}", option.name)?;
                }
            } else {
                for option_name in option_names {
                    named_options.insert(option_name.to_owned(), true);
                }
            }
        }

        for (option_name, value) in named_options {
            if option_name == "posix" {
                continue;
            }

            if let Some(option_def) = crate::engine::namedoptions::options(
                crate::engine::namedoptions::ShellOptionKind::SetO,
            )
            .get(option_name.as_str())
            {
                option_def.set(context.shell.options_mut(), value);
            } else {
                result = ExecutionResult::invalid_usage();
            }
        }

        let args = context.shell.current_shell_args_mut();

        let skip = match self.positional_args.first() {
            Some(x) if x == "-" => {
                if self.positional_args.len() > 1 {
                    args.clear();
                }
                1
            }
            Some(x) if x == "--" => {
                args.clear();
                1
            }
            Some(_) => {
                args.clear();
                0
            }
            None => 0,
        };

        for arg in self.positional_args.iter().skip(skip) {
            args.push(arg.to_owned());
        }

        saw_option = saw_option || !self.positional_args.is_empty();

        // If we *still* haven't seen any options, then we need to display all variables and
        // functions.
        if !saw_option {
            display_all(&context)?;
        }

        Ok(result)
    }
}

fn display_all(context: &crate::engine::ExecutionContext<'_>) -> Result<(), crate::engine::Error> {
    // Display variables.
    for (name, var) in context.shell.env().iter().sorted_by_key(|v| v.0) {
        if !var.is_enumerable() {
            continue;
        }

        // TODO(set): For now, skip all dynamic variables. The current behavior
        // of bash is not quite clear. We've empirically found that some
        // special variables don't get displayed until they're observed
        // at least once.
        if matches!(var.value(), variables::ShellValue::Dynamic(_)) {
            continue;
        }

        // Skip variables that have been declared but are unset.
        if !var.value().is_set() {
            continue;
        }

        writeln!(
            context.stdout(),
            "{name}={}",
            var.value()
                .format(variables::FormatStyle::Basic, context.shell)?,
        )?;
    }

    // Display functions.
    for (_name, registration) in context.shell.funcs().iter().sorted_by_key(|v| v.0) {
        writeln!(context.stdout(), "{}", registration.definition())?;
    }

    Ok(())
}
