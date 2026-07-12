use itertools::Itertools;
use std::io::Write;

use super::common::PlusMinusFlag;
use crate::engine::{
    ErrorKind, ExecutionResult, builtins,
    env::{self, EnvironmentLookup, EnvironmentScope},
    error,
    parser::ast,
    variables::{
        self, ArrayLiteral, ShellValue, ShellValueLiteral, ShellValueUnsetType, ShellVariable,
        ShellVariableUpdateTransform,
    },
};

/// Display or update variables and their attributes.
#[derive(Default)]
pub(crate) struct DeclareCommand {
    /// Constrain to function names or definitions.
    function_names_or_defs_only: bool,

    /// Constrain to function names only.
    function_names_only: bool,

    /// Create global variable, if applicable.
    create_global: bool,

    /// When creating a local variable that shadows another variable of the same name,
    /// then initialize it with the contents and attributes of the variable being shadowed.
    locals_inherit_from_prev_scope: bool,

    /// Display each item's attributes and values.
    print: bool,

    //
    // Attribute options
    make_indexed_array: PlusMinusFlag,
    make_associative_array: PlusMinusFlag,
    capitalize_value_on_assignment: PlusMinusFlag,
    make_integer: PlusMinusFlag,
    lowercase_value_on_assignment: PlusMinusFlag,
    make_nameref: PlusMinusFlag,
    make_readonly: PlusMinusFlag,
    make_traced: PlusMinusFlag,
    uppercase_value_on_assignment: PlusMinusFlag,
    make_exported: PlusMinusFlag,

    //
    // Declarations
    //
    declarations: Vec<crate::engine::CommandArg>,
}

#[derive(Clone, Copy)]
enum DeclareVerb {
    Declare,
    Local,
    Readonly,
}

impl builtins::DeclarationCommand for DeclareCommand {
    fn set_declarations(&mut self, declarations: Vec<crate::engine::CommandArg>) {
        self.declarations = declarations;
    }
}

fn parse_declare_args<I>(args: I) -> Result<DeclareCommand, String>
where
    I: IntoIterator<Item = String>,
{
    let mut command = DeclareCommand::default();
    let mut args = builtins::BuiltinArgs::new(args);

    while let Some(arg) = args.next_arg() {
        if arg == "--" {
            break;
        }

        if let Some(flags) = arg.strip_prefix('-') {
            if flags.is_empty() {
                return Err("declare: -: invalid option".into());
            }
            parse_declare_flags(&mut command, flags, true)?;
        } else if let Some(flags) = arg.strip_prefix('+') {
            if flags.is_empty() {
                return Err("declare: +: invalid option".into());
            }
            parse_declare_flags(&mut command, flags, false)?;
        } else {
            return Err(format!("declare: {arg}: invalid option"));
        }
    }

    Ok(command)
}

fn parse_declare_flags(
    command: &mut DeclareCommand,
    flags: &str,
    enabled: bool,
) -> Result<(), String> {
    for flag in flags.chars() {
        match flag {
            'f' if enabled => command.function_names_or_defs_only = true,
            'F' if enabled => command.function_names_only = true,
            'g' if enabled => command.create_global = true,
            'I' if enabled => command.locals_inherit_from_prev_scope = true,
            'p' if enabled => command.print = true,
            'a' => command.make_indexed_array.set(enabled),
            'A' => command.make_associative_array.set(enabled),
            'c' => command.capitalize_value_on_assignment.set(enabled),
            'i' => command.make_integer.set(enabled),
            'l' => command.lowercase_value_on_assignment.set(enabled),
            'n' => command.make_nameref.set(enabled),
            'r' => command.make_readonly.set(enabled),
            't' => command.make_traced.set(enabled),
            'u' => command.uppercase_value_on_assignment.set(enabled),
            'x' => command.make_exported.set(enabled),
            _ => {
                let prefix = if enabled { '-' } else { '+' };
                return Err(format!("declare: {prefix}{flag}: invalid option"));
            }
        }
    }
    Ok(())
}

impl builtins::Command for DeclareCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        parse_declare_args(args)
    }

    async fn execute(
        &self,
        mut context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let verb = match context.command_name.as_str() {
            "local" => DeclareVerb::Local,
            "readonly" => DeclareVerb::Readonly,
            _ => DeclareVerb::Declare,
        };

        if matches!(verb, DeclareVerb::Local) && !context.shell.in_function() {
            writeln!(context.stderr(), "can only be used in a function")?;
            return Ok(ExecutionResult::general_error());
        }

        if self.locals_inherit_from_prev_scope {
            return error::unimp("declare -I");
        }

        let mut result = ExecutionResult::success();
        if !self.declarations.is_empty() {
            for declaration in &self.declarations {
                if self.print && !matches!(verb, DeclareVerb::Readonly) {
                    if !self.try_display_declaration(&context, declaration, verb)? {
                        result = ExecutionResult::general_error();
                    }
                } else {
                    if !self.process_declaration(&mut context, declaration, verb)? {
                        result = ExecutionResult::general_error();
                    }
                }
            }
        } else {
            // Display matching declarations from the variable environment.
            if !self.function_names_only && !self.function_names_or_defs_only {
                self.display_matching_env_declarations(&context, verb)?;
            }

            // Do the same for functions.
            if !matches!(verb, DeclareVerb::Local | DeclareVerb::Readonly)
                && (!self.print || self.function_names_only || self.function_names_or_defs_only)
            {
                self.display_matching_functions(&context)?;
            }
        }

        Ok(result)
    }
}

impl DeclareCommand {
    fn try_display_declaration(
        &self,
        context: &crate::engine::ExecutionContext<'_>,
        declaration: &crate::engine::CommandArg,
        verb: DeclareVerb,
    ) -> Result<bool, crate::engine::Error> {
        let name = match declaration {
            crate::engine::CommandArg::String(s) => s,
            crate::engine::CommandArg::Assignment(_) => {
                writeln!(context.stderr(), "declare: {declaration}: not found")?;
                return Ok(false);
            }
        };

        let lookup = if matches!(verb, DeclareVerb::Local) {
            EnvironmentLookup::OnlyInCurrentLocal
        } else {
            EnvironmentLookup::Anywhere
        };

        if self.function_names_only || self.function_names_or_defs_only {
            if let Some(func_registration) = context.shell.funcs().get(name) {
                if self.function_names_only {
                    if self.print {
                        writeln!(context.stdout(), "declare -f {name}")?;
                    } else {
                        writeln!(context.stdout(), "{name}")?;
                    }
                } else {
                    writeln!(context.stdout(), "{}", func_registration.definition())?;
                }
                Ok(true)
            } else {
                // For some reason, bash does not print an error message in this case.
                Ok(false)
            }
        } else if let Some(variable) = context.shell.env().get_using_policy(name, lookup) {
            let mut cs = variable.attribute_flags(context.shell);
            if cs.is_empty() {
                cs.push('-');
            }

            let resolved_value = variable.resolve_value(context.shell);
            let separator_str = if matches!(resolved_value, ShellValue::Unset(_)) {
                ""
            } else {
                "="
            };

            writeln!(
                context.stdout(),
                "declare -{cs} {name}{separator_str}{}",
                resolved_value.format(variables::FormatStyle::DeclarePrint, context.shell)?
            )?;

            Ok(true)
        } else {
            writeln!(context.stderr(), "declare: {name}: not found")?;
            Ok(false)
        }
    }

    fn process_declaration(
        &self,
        context: &mut crate::engine::ExecutionContext<'_>,
        declaration: &crate::engine::CommandArg,
        verb: DeclareVerb,
    ) -> Result<bool, crate::engine::Error> {
        let create_var_local = matches!(verb, DeclareVerb::Local)
            || (matches!(verb, DeclareVerb::Declare)
                && context.shell.in_function()
                && !self.create_global);

        if self.function_names_or_defs_only || self.function_names_only {
            return self.try_display_declaration(context, declaration, verb);
        }

        // Extract the variable name and the initial value being assigned (if any).
        let (name, assigned_index, initial_value, name_is_array, append) =
            Self::declaration_to_name_and_value(declaration)?;

        // Special-case: `local -`
        if name == "-" && matches!(verb, DeclareVerb::Local) {
            return error::unimp("local -");
        }

        // Make sure it's a valid name.
        if !env::valid_variable_name(name.as_str()) {
            writeln!(
                context.stderr(),
                "{}: {name}: not a valid variable name",
                context.command_name
            )?;
            return Ok(false);
        }

        // Figure out where we should look.
        let lookup = declaration_lookup(create_var_local, self.create_global);

        let (initial_value, integer_append_evaluated) = self.evaluate_integer_assignment(
            context.shell,
            name.as_str(),
            lookup,
            initial_value,
            append,
        )?;

        // Look up the variable.
        if let Some(var) = context
            .shell
            .env_mut()
            .get_mut_using_policy(name.as_str(), lookup)
        {
            if self.make_associative_array.is_some() {
                var.convert_to_associative_array()?;
            }
            if self.make_indexed_array.is_some() {
                var.convert_to_indexed_array()?;
            }

            self.apply_attributes_before_update(var)?;

            if let Some(initial_value) = initial_value {
                let append = !integer_append_evaluated && (append || assigned_index.is_some());
                var.assign(initial_value, append)?;
            }

            self.apply_attributes_after_update(var, verb)?;
        } else {
            let unset_type = if self.make_indexed_array.is_some() {
                ShellValueUnsetType::IndexedArray
            } else if self.make_associative_array.is_some() {
                ShellValueUnsetType::AssociativeArray
            } else if name_is_array {
                ShellValueUnsetType::IndexedArray
            } else {
                ShellValueUnsetType::Untyped
            };

            let mut var = ShellVariable::new(ShellValue::Unset(unset_type));

            self.apply_attributes_before_update(&mut var)?;

            if let Some(initial_value) = initial_value {
                var.assign(initial_value, false)?;
            }

            if context.shell.options().export_variables_on_modification && !var.value().is_array() {
                var.export();
            }

            self.apply_attributes_after_update(&mut var, verb)?;

            let scope = if create_var_local {
                EnvironmentScope::Local
            } else {
                EnvironmentScope::Global
            };

            context.shell.env_mut().add(name, var, scope)?;
        }

        Ok(true)
    }

    #[expect(clippy::type_complexity)]
    fn declaration_to_name_and_value(
        declaration: &crate::engine::CommandArg,
    ) -> Result<
        (
            String,
            Option<String>,
            Option<ShellValueLiteral>,
            bool,
            bool,
        ),
        crate::engine::Error,
    > {
        let name;
        let assigned_index;
        let initial_value;
        let name_is_array;
        let append;

        match declaration {
            crate::engine::CommandArg::String(s) => {
                // We need to handle the case of someone invoking `declare array[index]`.
                // In such case, we ignore the index and treat it as a declaration of
                // the array.
                if let Some(without_closing_bracket) = s.strip_suffix(']')
                    && let Some(open_bracket) = without_closing_bracket.find('[')
                {
                    name = without_closing_bracket[..open_bracket].to_owned();
                    assigned_index = Some(without_closing_bracket[open_bracket + 1..].to_owned());
                    name_is_array = true;
                } else {
                    name = s.clone();
                    assigned_index = None;
                    name_is_array = false;
                }
                initial_value = None;
                append = false;
            }
            crate::engine::CommandArg::Assignment(assignment) => {
                append = assignment.append;
                match &assignment.name {
                    ast::AssignmentName::VariableName(var_name) => {
                        name = var_name.to_owned();
                        assigned_index = None;
                    }
                    ast::AssignmentName::ArrayElementName(var_name, index) => {
                        if matches!(assignment.value, ast::AssignmentValue::Array(_)) {
                            return Err(ErrorKind::AssigningListToArrayMember.into());
                        }

                        name = var_name.to_owned();
                        assigned_index = Some(index.to_owned());
                    }
                }

                match &assignment.value {
                    ast::AssignmentValue::Scalar(s) => {
                        if let Some(index) = &assigned_index {
                            initial_value = Some(ShellValueLiteral::Array(ArrayLiteral(vec![(
                                Some(index.to_owned()),
                                s.value.clone(),
                            )])));
                            name_is_array = true;
                        } else {
                            initial_value = Some(ShellValueLiteral::Scalar(s.value.clone()));
                            name_is_array = false;
                        }
                    }
                    ast::AssignmentValue::Array(a) => {
                        initial_value = Some(ShellValueLiteral::Array(ArrayLiteral(
                            a.iter()
                                .map(|(i, v)| {
                                    (i.as_ref().map(|w| w.value.clone()), v.value.clone())
                                })
                                .collect(),
                        )));
                        name_is_array = true;
                    }
                }
            }
        }

        Ok((name, assigned_index, initial_value, name_is_array, append))
    }

    fn evaluate_integer_assignment(
        &self,
        shell: &mut crate::engine::Shell,
        name: &str,
        lookup: EnvironmentLookup,
        value: Option<ShellValueLiteral>,
        append: bool,
    ) -> Result<(Option<ShellValueLiteral>, bool), crate::engine::Error> {
        let Some(value) = value else {
            return Ok((None, false));
        };
        let integer_enabled = self.make_integer.to_bool().unwrap_or_else(|| {
            shell
                .env()
                .get_using_policy(name, lookup)
                .is_some_and(ShellVariable::is_treated_as_integer)
        });
        if !integer_enabled {
            return Ok((Some(value), false));
        }

        let ShellValueLiteral::Scalar(expression) = value else {
            return error::unimp("integer array declaration assignment");
        };
        let right = crate::engine::arithmetic::eval_str(shell, expression.as_str())?;
        let result = if append {
            let left_expression = shell
                .env()
                .get_using_policy(name, lookup)
                .and_then(|variable| {
                    variable
                        .resolve_value(shell)
                        .try_get_cow_str(shell)
                        .map(|value| value.into_owned())
                })
                .unwrap_or_else(|| "0".to_owned());
            let left = crate::engine::arithmetic::eval_str(shell, left_expression.as_str())?;
            left.wrapping_add(right)
        } else {
            right
        };

        Ok((Some(ShellValueLiteral::Scalar(result.to_string())), append))
    }

    fn display_matching_env_declarations(
        &self,
        context: &crate::engine::ExecutionContext<'_>,
        verb: DeclareVerb,
    ) -> Result<(), crate::engine::Error> {
        //
        // Dump all declarations. Use attribute flags to filter which variables are dumped.
        //

        // We start by excluding all variables that are not enumerable.
        #[expect(clippy::type_complexity)]
        let mut filters: Vec<Box<dyn Fn((&String, &ShellVariable)) -> bool>> =
            vec![Box::new(|(_, v)| v.is_enumerable())];

        // Add filters depending on verb.
        if matches!(verb, DeclareVerb::Readonly) {
            filters.push(Box::new(|(_, v)| v.is_readonly()));
        }

        // Add filters depending on attribute flags.
        if let Some(value) = self.make_indexed_array.to_bool() {
            filters.push(Box::new(move |(_, v)| {
                matches!(v.value(), ShellValue::IndexedArray(_)) == value
            }));
        }
        if let Some(value) = self.make_associative_array.to_bool() {
            filters.push(Box::new(move |(_, v)| {
                matches!(v.value(), ShellValue::AssociativeArray(_)) == value
            }));
        }
        if let Some(value) = self.make_integer.to_bool() {
            filters.push(Box::new(move |(_, v)| v.is_treated_as_integer() == value));
        }
        if let Some(value) = self.capitalize_value_on_assignment.to_bool() {
            filters.push(Box::new(move |(_, v)| {
                matches!(
                    v.get_update_transform(),
                    ShellVariableUpdateTransform::Capitalize
                ) == value
            }));
        }
        if let Some(value) = self.lowercase_value_on_assignment.to_bool() {
            filters.push(Box::new(move |(_, v)| {
                matches!(
                    v.get_update_transform(),
                    ShellVariableUpdateTransform::Lowercase
                ) == value
            }));
        }
        if let Some(value) = self.make_nameref.to_bool() {
            filters.push(Box::new(move |(_, v)| v.is_treated_as_nameref() == value));
        }
        if let Some(value) = self.make_readonly.to_bool() {
            filters.push(Box::new(move |(_, v)| v.is_readonly() == value));
        }
        if let Some(value) = self.make_traced.to_bool() {
            filters.push(Box::new(move |(_, v)| trace_attribute_matches(v, value)));
        }
        if let Some(value) = self.uppercase_value_on_assignment.to_bool() {
            filters.push(Box::new(move |(_, v)| {
                matches!(
                    v.get_update_transform(),
                    ShellVariableUpdateTransform::Uppercase
                ) == value
            }));
        }
        if let Some(value) = self.make_exported.to_bool() {
            filters.push(Box::new(move |(_, v)| v.is_exported() == value));
        }

        let iter_policy = if matches!(verb, DeclareVerb::Local) {
            EnvironmentLookup::OnlyInCurrentLocal
        } else {
            EnvironmentLookup::Anywhere
        };

        // Iterate through an ordered list of all matching declarations tracked in the
        // environment.
        for (name, variable) in context
            .shell
            .env()
            .iter_using_policy(iter_policy)
            .filter(|pair| filters.iter().all(|f| f(*pair)))
            .sorted_by_key(|v| v.0)
        {
            if self.print {
                let mut cs = variable.attribute_flags(context.shell);
                if cs.is_empty() {
                    cs.push('-');
                }

                let separator_str = if matches!(variable.value(), ShellValue::Unset(_)) {
                    ""
                } else {
                    "="
                };

                writeln!(
                    context.stdout(),
                    "declare -{cs} {name}{separator_str}{}",
                    variable
                        .value()
                        .format(variables::FormatStyle::DeclarePrint, context.shell)?
                )?;
            } else {
                writeln!(
                    context.stdout(),
                    "{name}={}",
                    variable
                        .value()
                        .format(variables::FormatStyle::Basic, context.shell)?
                )?;
            }
        }

        Ok(())
    }

    fn display_matching_functions(
        &self,
        context: &crate::engine::ExecutionContext<'_>,
    ) -> Result<(), crate::engine::Error> {
        for (name, registration) in context.shell.funcs().iter().sorted_by_key(|v| v.0) {
            if self.function_names_only {
                writeln!(context.stdout(), "declare -f {name}")?;
            } else {
                writeln!(context.stdout(), "{}", registration.definition())?;
            }
        }

        Ok(())
    }

    #[expect(clippy::unnecessary_wraps)]
    const fn apply_attributes_before_update(
        &self,
        var: &mut ShellVariable,
    ) -> Result<(), crate::engine::Error> {
        if let Some(value) = self.make_integer.to_bool() {
            if value {
                var.treat_as_integer();
            } else {
                var.unset_treat_as_integer();
            }
        }
        if let Some(value) = self.capitalize_value_on_assignment.to_bool() {
            if value {
                var.set_update_transform(ShellVariableUpdateTransform::Capitalize);
            } else if matches!(
                var.get_update_transform(),
                ShellVariableUpdateTransform::Capitalize
            ) {
                var.set_update_transform(ShellVariableUpdateTransform::None);
            }
        }
        if let Some(value) = self.lowercase_value_on_assignment.to_bool() {
            if value {
                var.set_update_transform(ShellVariableUpdateTransform::Lowercase);
            } else if matches!(
                var.get_update_transform(),
                ShellVariableUpdateTransform::Lowercase
            ) {
                var.set_update_transform(ShellVariableUpdateTransform::None);
            }
        }
        if let Some(value) = self.make_nameref.to_bool() {
            if value {
                var.treat_as_nameref();
            } else {
                var.unset_treat_as_nameref();
            }
        }
        if let Some(value) = self.make_traced.to_bool() {
            if value {
                var.enable_trace();
            } else {
                var.disable_trace();
            }
        }
        if let Some(value) = self.uppercase_value_on_assignment.to_bool() {
            if value {
                var.set_update_transform(ShellVariableUpdateTransform::Uppercase);
            } else if matches!(
                var.get_update_transform(),
                ShellVariableUpdateTransform::Uppercase
            ) {
                var.set_update_transform(ShellVariableUpdateTransform::None);
            }
        }
        if let Some(value) = self.make_exported.to_bool() {
            if value {
                var.export();
            } else {
                var.unexport();
            }
        }

        Ok(())
    }

    fn apply_attributes_after_update(
        &self,
        var: &mut ShellVariable,
        verb: DeclareVerb,
    ) -> Result<(), crate::engine::Error> {
        if matches!(verb, DeclareVerb::Readonly) {
            var.set_readonly();
        } else if let Some(value) = self.make_readonly.to_bool() {
            if value {
                var.set_readonly();
            } else {
                var.unset_readonly()?;
            }
        }

        Ok(())
    }
}

fn trace_attribute_matches(variable: &ShellVariable, expected: bool) -> bool {
    variable.is_trace_enabled() == expected
}

fn declaration_lookup(create_var_local: bool, create_global: bool) -> EnvironmentLookup {
    if create_global {
        EnvironmentLookup::OnlyInGlobal
    } else if create_var_local {
        EnvironmentLookup::OnlyInCurrentLocal
    } else {
        EnvironmentLookup::Anywhere
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declare_g_uses_global_scope_even_when_a_local_exists() {
        assert!(matches!(
            declaration_lookup(false, true),
            EnvironmentLookup::OnlyInGlobal
        ));
    }

    #[test]
    fn trace_filter_comes_from_trace_flag() {
        let command = parse_declare_args(["declare", "-t"].map(String::from)).unwrap();
        assert_eq!(command.make_traced.to_bool(), Some(true));
        assert_eq!(command.make_readonly.to_bool(), None);

        let mut variable = ShellVariable::new(ShellValue::Unset(ShellValueUnsetType::Untyped));
        assert!(!trace_attribute_matches(&variable, true));
        variable.enable_trace();
        assert!(trace_attribute_matches(&variable, true));

        let command = parse_declare_args(["declare", "-t", "+t"].map(String::from)).unwrap();
        assert_eq!(command.make_traced.to_bool(), Some(false));
    }

    #[compio::test]
    async fn declare_g_updates_the_global_hidden_by_a_local() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        let result = shell
            .run_string(
                "GLOBAL_TEST=before; f() { local GLOBAL_TEST=local; declare -g GLOBAL_TEST=after; }; f",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        assert!(result.is_success());
        let variable = shell
            .env()
            .get_using_policy("GLOBAL_TEST", EnvironmentLookup::OnlyInGlobal)
            .unwrap();
        assert_eq!(
            variable.value().try_get_cow_str(&shell).as_deref(),
            Some("after")
        );
        Ok(())
    }

    #[compio::test]
    async fn nameref_assignment_expansion_and_unset_follow_target() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        let result = shell
            .run_string(
                "target=before; declare -n ref=target; ref=after",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;
        assert!(result.is_success());
        assert_eq!(shell.env_str("target").as_deref(), Some("after"));
        assert_eq!(shell.env_str("ref").as_deref(), Some("after"));
        assert!(
            shell
                .env()
                .get_using_policy("ref", EnvironmentLookup::Anywhere)
                .unwrap()
                .is_treated_as_nameref()
        );

        let result = shell
            .run_string(
                "unset -n ref",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;
        assert!(result.is_success());
        assert!(shell.env().get("ref").is_none());
        assert_eq!(shell.env_str("target").as_deref(), Some("after"));

        shell
            .run_string(
                "declare -n ref=target; unset ref",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;
        assert!(shell.env().get("ref").is_some());
        assert!(shell.env().get("target").is_none());
        Ok(())
    }

    #[compio::test]
    async fn nameref_expansion_supports_missing_and_array_element_targets() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        shell
            .run_string(
                "declare -n missing_ref=created; missing_ref=value; array[2]=two; declare -n element_ref='array[2]'",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        assert_eq!(shell.env_str("created").as_deref(), Some("value"));
        assert_eq!(shell.env_str("element_ref").as_deref(), Some("two"));
        let expanded =
            crate::engine::expansion::basic_expand_word(&mut shell, &params, "${element_ref}")
                .await?;
        assert_eq!(expanded, "two");
        Ok(())
    }

    #[compio::test]
    async fn nameref_cycles_fail_expansion() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        shell
            .run_string(
                "declare -n a=b; declare -n b=a",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        let error = crate::engine::expansion::basic_expand_word(&mut shell, &params, "$a")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("nameref cycle"));
        Ok(())
    }

    #[compio::test]
    async fn declare_integer_evaluates_expressions_bases_variables_and_append() -> anyhow::Result<()>
    {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        let result = shell
            .run_string(
                "base=4; declare -i x=1+2; declare -i y=base*3; declare -i radix=16#ff; declare -i x+=2#10",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        assert!(result.is_success());
        assert_eq!(shell.env_str("x").as_deref(), Some("5"));
        assert_eq!(shell.env_str("y").as_deref(), Some("12"));
        assert_eq!(shell.env_str("radix").as_deref(), Some("255"));
        Ok(())
    }
}
