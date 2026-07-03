use clap::Parser;
use std::io::Write;

use crate::engine::{ErrorKind, ExecutionParameters, ExecutionResult, Shell, builtins, tests};

/// Evaluate test expression.
#[derive(Parser)]
#[clap(disable_help_flag = true, disable_version_flag = true)]
pub(crate) struct TestCommand {
    #[clap(allow_hyphen_values = true)]
    args: Vec<String>,
}

impl builtins::Command for TestCommand {
    type Error = crate::engine::Error;

    fn arg_parsing() -> builtins::ArgParsing {
        builtins::ArgParsing::PreserveDoubleDashRest
    }

    fn append_rest_args(&mut self, rest: Vec<String>) {
        self.args.extend(rest);
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
