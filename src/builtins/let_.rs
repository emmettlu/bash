use std::io::Write;

use crate::engine::{ExecutionResult, arithmetic::Evaluatable, builtins};

/// Evaluate arithmetic expressions.
pub(crate) struct LetCommand {
    /// Arithmetic expressions to evaluate.
    exprs: Vec<String>,
}

impl builtins::Command for LetCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let args = builtins::BuiltinArgs::new(args);
        Ok(Self { exprs: args.rest() })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut result = ExecutionResult::invalid_usage();

        if self.exprs.is_empty() {
            writeln!(context.stderr(), "missing expression")?;
            return Ok(result);
        }

        for expr in &self.exprs {
            let parsed = crate::parser::arithmetic::parse(expr.as_str())?;
            let evaluated = parsed.eval(context.shell)?;

            if evaluated == 0 {
                result = ExecutionResult::general_error();
            } else {
                result = ExecutionResult::success();
            }
        }

        Ok(result)
    }
}
