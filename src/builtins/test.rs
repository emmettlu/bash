use std::io::Write;

use crate::engine::{ErrorKind, ExecutionParameters, ExecutionResult, Shell, builtins, tests};

/// Evaluate test expression.
pub(crate) struct TestCommand {
    args: Vec<String>,
}

impl builtins::Command for TestCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {
            args: builtins::BuiltinArgs::new(args).rest(),
        })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut args = self.args.as_slice();

        if context.command_name == "[" {
            match args.last() {
                Some(s) if s == "]" => (),
                None | Some(_) => {
                    writeln!(context.stderr(), "[: missing ']'")?;
                    return Ok(ExecutionResult::invalid_usage());
                }
            }

            args = &args[0..args.len() - 1];
        }

        if execute_test(context.shell, &context.params, args)? {
            Ok(ExecutionResult::success())
        } else {
            Ok(ExecutionResult::general_error())
        }
    }
}

fn execute_test(
    shell: &mut Shell,
    params: &ExecutionParameters,
    args: &[String],
) -> Result<bool, crate::engine::Error> {
    let test_command =
        crate::parser::test_command::parse(args).map_err(ErrorKind::TestCommandParseError)?;
    tests::eval_expr(&test_command, shell, params)
}
