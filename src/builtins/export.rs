use itertools::Itertools;
use std::io::Write;

use crate::engine::{
    CommandArg, ExecutionResult, builtins,
    env::{EnvironmentLookup, EnvironmentScope},
    escape,
    parser::ast,
    variables::{self, ShellValue, ShellValueUnsetType, ShellVariable},
};

/// Add or update exported shell variables.
pub(crate) struct ExportCommand {
    /// Names are treated as function names.
    names_are_functions: bool,

    /// Un-export the names.
    unexport: bool,

    /// 仅显示 export 声明, 不修改变量或函数。
    print: bool,

    //
    // Declarations
    //
    declarations: Vec<CommandArg>,
}

impl builtins::DeclarationCommand for ExportCommand {
    fn set_declarations(&mut self, declarations: Vec<CommandArg>) {
        self.declarations = declarations;
    }
}

impl builtins::Command for ExportCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let mut command = Self {
            names_are_functions: false,
            unexport: false,
            print: false,
            declarations: Vec::new(),
        };

        let positionals = args.parse_flags(|flag| match flag {
            'f' => {
                command.names_are_functions = true;
                Ok(true)
            }
            'n' => {
                command.unexport = true;
                Ok(true)
            }
            'p' => {
                command.print = true;
                Ok(true)
            }
            _ => Err(format!("export: -{flag}: invalid option")),
        })?;

        if !positionals.is_empty() {
            return Err(format!("export: {}: unexpected argument", positionals[0]));
        }

        Ok(command)
    }

    async fn execute(
        &self,
        mut context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.declarations.is_empty() {
            if self.names_are_functions {
                display_all_exported_functions(&context)?;
            } else {
                display_all_exported_vars(&context)?;
            }
            return Ok(ExecutionResult::success());
        }

        let mut result = ExecutionResult::success();
        for decl in &self.declarations {
            let current_result = self.process_decl(&mut context, decl)?;
            if !current_result.is_success() {
                result = current_result;
            }
        }

        Ok(result)
    }
}

impl ExportCommand {
    fn process_decl(
        &self,
        context: &mut crate::engine::ExecutionContext<'_>,
        decl: &CommandArg,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        match decl {
            CommandArg::String(s) => {
                if self.names_are_functions {
                    if self.print {
                        if !display_exported_function(context, s)? {
                            writeln!(context.stderr(), "{s}: not found")?;
                            return Ok(ExecutionResult::general_error());
                        }
                    } else if let Some(func) = context.shell.func_mut(s) {
                        if self.unexport {
                            func.unexport();
                        } else {
                            func.export();
                        }
                    } else {
                        writeln!(context.stderr(), "{s}: not a function")?;
                        return Ok(ExecutionResult::invalid_usage());
                    }
                } else if self.print {
                    if !display_exported_var(context, s)? {
                        writeln!(context.stderr(), "{s}: not found")?;
                        return Ok(ExecutionResult::general_error());
                    }
                } else if let Some((_, variable)) = context.shell.env_mut().get_mut(s) {
                    if self.unexport {
                        variable.unexport();
                    } else {
                        variable.export();
                    }
                } else if !self.unexport {
                    let mut variable =
                        ShellVariable::new(ShellValue::Unset(ShellValueUnsetType::Untyped));
                    variable.export();
                    context
                        .shell
                        .env_mut()
                        .add(s, variable, EnvironmentScope::Global)?;
                }
            }
            CommandArg::Assignment(assignment) => {
                if self.print || self.names_are_functions {
                    writeln!(context.stderr(), "{decl}: not found")?;
                    return Ok(ExecutionResult::general_error());
                }

                let name = match &assignment.name {
                    ast::AssignmentName::VariableName(name) => name,
                    ast::AssignmentName::ArrayElementName(_, _) => {
                        writeln!(context.stderr(), "not a valid variable name")?;
                        return Ok(ExecutionResult::invalid_usage());
                    }
                };

                let value = match &assignment.value {
                    ast::AssignmentValue::Scalar(s) => {
                        variables::ShellValueLiteral::Scalar(s.flatten())
                    }
                    ast::AssignmentValue::Array(a) => {
                        variables::ShellValueLiteral::Array(variables::ArrayLiteral(
                            a.iter()
                                .map(|(k, v)| (k.as_ref().map(|k| k.flatten()), v.flatten()))
                                .collect(),
                        ))
                    }
                };

                // Update the variable with the provided value and then mark it exported.
                context.shell.env_mut().update_or_add(
                    name,
                    value,
                    |var| {
                        if self.unexport {
                            var.unexport();
                        } else {
                            var.export();
                        }
                        Ok(())
                    },
                    EnvironmentLookup::Anywhere,
                    EnvironmentScope::Global,
                )?;
            }
        }

        Ok(ExecutionResult::success())
    }
}

fn display_all_exported_vars(
    context: &crate::engine::ExecutionContext<'_>,
) -> Result<(), crate::engine::Error> {
    for (name, variable) in context.shell.env().iter().sorted_by_key(|v| v.0) {
        if variable.is_exported() {
            write_exported_var(context, name, variable)?;
        }
    }

    Ok(())
}

fn display_exported_var(
    context: &crate::engine::ExecutionContext<'_>,
    name: &str,
) -> Result<bool, crate::engine::Error> {
    let Some(variable) = context
        .shell
        .env()
        .get_using_policy(name, EnvironmentLookup::Anywhere)
        .filter(|variable| variable.is_exported())
    else {
        return Ok(false);
    };

    write_exported_var(context, name, variable)?;
    Ok(true)
}

fn write_exported_var(
    context: &crate::engine::ExecutionContext<'_>,
    name: &str,
    variable: &ShellVariable,
) -> Result<(), crate::engine::Error> {
    let value = variable.value().try_get_cow_str(context.shell);
    writeln!(
        context.stdout(),
        "{}",
        format_export_declaration(name, value.as_deref())
    )?;
    Ok(())
}

fn format_export_declaration(name: &str, value: Option<&str>) -> String {
    value.map_or_else(
        || format!("declare -x {name}"),
        |value| {
            format!(
                "declare -x {name}={}",
                escape::force_quote(value, escape::QuoteMode::SingleQuote)
            )
        },
    )
}

fn display_all_exported_functions(
    context: &crate::engine::ExecutionContext<'_>,
) -> Result<(), crate::engine::Error> {
    for (name, function) in context.shell.funcs().iter().sorted_by_key(|item| item.0) {
        if function.is_exported() {
            write_exported_function(context, name, function)?;
        }
    }
    Ok(())
}

fn display_exported_function(
    context: &crate::engine::ExecutionContext<'_>,
    name: &str,
) -> Result<bool, crate::engine::Error> {
    let Some(function) = context
        .shell
        .funcs()
        .get(name)
        .filter(|function| function.is_exported())
    else {
        return Ok(false);
    };

    write_exported_function(context, name, function)?;
    Ok(true)
}

fn write_exported_function(
    context: &crate::engine::ExecutionContext<'_>,
    name: &str,
    function: &crate::engine::functions::Registration,
) -> Result<(), crate::engine::Error> {
    writeln!(context.stdout(), "{}", function.definition())?;
    writeln!(
        context.stdout(),
        "export -f {}",
        escape::quote_if_needed(name, escape::QuoteMode::SingleQuote)
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_print_and_function_options() {
        let command =
            <ExportCommand as builtins::Command>::new(["export", "-pf"].map(String::from)).unwrap();
        assert!(command.print);
        assert!(command.names_are_functions);
    }

    #[test]
    fn exported_values_are_replayably_escaped() {
        assert_eq!(
            format_export_declaration("VALUE", Some("a b")),
            "declare -x VALUE='a b'"
        );
        assert_eq!(
            format_export_declaration("EMPTY", Some("")),
            "declare -x EMPTY=''"
        );
    }

    #[compio::test]
    async fn exporting_a_missing_variable_creates_an_unset_export() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        let result = shell
            .run_string(
                "export CREATED_BY_TEST",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        assert!(result.is_success());
        let variable = shell
            .env()
            .get_using_policy("CREATED_BY_TEST", EnvironmentLookup::Anywhere)
            .unwrap();
        assert!(variable.is_exported());
        assert!(matches!(variable.value(), ShellValue::Unset(_)));
        Ok(())
    }

    #[compio::test]
    async fn export_p_reports_a_missing_variable_without_creating_it() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        let result = shell
            .run_string(
                "export -p MISSING_EXPORT_TEST",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        assert!(!result.is_success());
        assert!(
            shell
                .env()
                .get_using_policy("MISSING_EXPORT_TEST", EnvironmentLookup::Anywhere)
                .is_none()
        );
        Ok(())
    }

    #[compio::test]
    async fn export_f_marks_a_function_exported() -> anyhow::Result<()> {
        let mut shell = crate::engine::Shell::builder()
            .builtins(crate::builtins::default_builtins())
            .build()
            .await?;
        let params = shell.default_exec_params();
        let result = shell
            .run_string(
                "export_test_fn() { :; }; export -f export_test_fn",
                &crate::engine::SourceInfo::from("(test)"),
                &params,
            )
            .await?;

        assert!(result.is_success());
        assert!(shell.funcs().get("export_test_fn").unwrap().is_exported());
        Ok(())
    }
}
