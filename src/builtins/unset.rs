use std::borrow::Cow;

use crate::engine::{ExecutionResult, Shell, ShellValue, builtins, variables::ShellValueUnsetType};

/// Unset a variable.
pub(crate) struct UnsetCommand {
    name_interpretation: UnsetNameInterpretation,

    /// Names of variables to unset.
    names: Vec<String>,
}

#[derive(Default)]
pub(crate) struct UnsetNameInterpretation {
    /// Treat each name as a shell function.
    shell_functions: bool,

    /// Treat each name as a shell variable.
    shell_variables: bool,

    /// Treat each name as a name reference.
    name_references: bool,
}

impl UnsetNameInterpretation {
    pub const fn unspecified(&self) -> bool {
        !self.shell_functions && !self.shell_variables && !self.name_references
    }

    const fn specified_count(&self) -> usize {
        self.shell_functions as usize
            + self.shell_variables as usize
            + self.name_references as usize
    }
}

impl builtins::Command for UnsetCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut command = Self {
            name_interpretation: UnsetNameInterpretation::default(),
            names: Vec::new(),
        };
        let mut args = builtins::BuiltinArgs::new(args);
        let names = args.parse_flags(|flag| match flag {
            'f' => {
                command.name_interpretation.shell_functions = true;
                Ok(true)
            }
            'v' => {
                command.name_interpretation.shell_variables = true;
                Ok(true)
            }
            'n' => {
                command.name_interpretation.name_references = true;
                Ok(true)
            }
            _ => Err(format!("unset: -{flag}: invalid option")),
        })?;
        command.names = names;

        if command.name_interpretation.specified_count() > 1 {
            return Err("unset: -f, -v, and -n are mutually exclusive".into());
        }

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let unspecified = self.name_interpretation.unspecified();

        #[expect(clippy::needless_continue)]
        for name in &self.names {
            if unspecified
                || self.name_interpretation.shell_variables
                || self.name_interpretation.name_references
            {
                // Try to parse the name as a parameter. If we can't, don't bail; it may not be a
                // valid variable name/parameter but could still be a function name.
                if let Ok(parameter) =
                    crate::parser::word::parse_parameter(name, &context.shell.parser_options())
                {
                    let result = match parameter {
                        crate::parser::word::Parameter::Positional(_) => continue,
                        crate::parser::word::Parameter::Special(_) => continue,
                        crate::parser::word::Parameter::Named(name) => {
                            if self.name_interpretation.name_references {
                                context.shell.env_mut().unset_raw(name.as_str())?.is_some()
                            } else {
                                context.shell.env_mut().unset(name.as_str())?.is_some()
                            }
                        }
                        crate::parser::word::Parameter::NamedWithIndex { name, index } => {
                            if self.name_interpretation.name_references {
                                continue;
                            }
                            unset_array_index(context.shell, name.as_str(), index.as_str())?
                        }
                        crate::parser::word::Parameter::NamedWithAllIndices {
                            name: _,
                            concatenate: _,
                        } => continue,
                    };

                    if result {
                        continue;
                    }
                }
            }

            // TODO(unset): Deal with readonly functions
            if (unspecified || self.name_interpretation.shell_functions)
                && context.shell.undefine_func(name)
            {
                continue;
            }
        }

        Ok(ExecutionResult::success())
    }
}

fn unset_array_index(
    shell: &mut Shell,
    name: &str,
    index: &str,
) -> Result<bool, crate::engine::Error> {
    let target = shell.env().resolve_target(name)?;
    if target.index.is_some() {
        return Err(crate::engine::ErrorKind::BadSubstitution(
            "combining an explicit array index with an array-element nameref is unsupported".into(),
        )
        .into());
    }

    // First check to see if it's an associative array.
    let is_assoc_array = if let Some((_, var)) = shell.env().get(target.name.as_str()) {
        matches!(
            var.value(),
            ShellValue::AssociativeArray(_)
                | ShellValue::Unset(ShellValueUnsetType::AssociativeArray)
        )
    } else {
        false
    };

    // Compute which index we should actually use. For indexed arrays, we need to evaluate
    // the index string as an arithmetic expression first.
    let index_to_use: Cow<'_, str> = if is_assoc_array {
        index.into()
    } else {
        // First evaluate the index expression.
        let index_as_expr = crate::parser::arithmetic::parse(index)?;
        let evaluated_index = shell.eval_arithmetic(&index_as_expr)?;
        evaluated_index.to_string().into()
    };

    // Now we can try to unset, and return the result.
    shell.env_mut().unset_index(name, index_to_use.as_ref())
}
