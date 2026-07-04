use itertools::Itertools;
use std::io::Write;

use crate::engine::{
    CommandArg, ExecutionResult, builtins,
    env::{EnvironmentLookup, EnvironmentScope},
    parser::ast,
    variables,
};

/// Add or update exported shell variables.
pub(crate) struct ExportCommand {
    /// Names are treated as function names.
    names_are_functions: bool,

    /// Un-export the names.
    unexport: bool,

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
            'p' => Ok(true),
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
            display_all_exported_vars(&context)?;
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
                // See if this is supposed to be a function name.
                if self.names_are_functions {
                    // Try to find the function already present; if we find it, then mark it
                    // exported.
                    if let Some(func) = context.shell.func_mut(s) {
                        if self.unexport {
                            func.unexport();
                        } else {
                            func.export();
                        }
                    } else {
                        writeln!(context.stderr(), "{s}: not a function")?;
                        return Ok(ExecutionResult::invalid_usage());
                    }
                }
                // Try to find the variable already present; if we find it, then mark it
                // exported.
                else if let Some((_, variable)) = context.shell.env_mut().get_mut(s) {
                    if self.unexport {
                        variable.unexport();
                    } else {
                        variable.export();
                    }
                }
            }
            CommandArg::Assignment(assignment) => {
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
    // Enumerate variables, sorted by key.
    for (name, variable) in context.shell.env().iter().sorted_by_key(|v| v.0) {
        if variable.is_exported() {
            let value = variable.value().try_get_cow_str(context.shell);
            if let Some(value) = value {
                writeln!(context.stdout(), "declare -x {name}=\"{value}\"")?;
            } else {
                writeln!(context.stdout(), "declare -x {name}")?;
            }
        }
    }

    Ok(())
}
