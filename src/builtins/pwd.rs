use crate::engine::{ExecutionResult, builtins, sys};
use std::{borrow::Cow, io::Write, path::Path};

/// Display the current working directory.
pub(crate) struct PwdCommand {
    /// Print the physical directory without any symlinks.
    physical: bool,
}

impl builtins::Command for PwdCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut physical = false;
        let mut args = builtins::BuiltinArgs::new(args);
        let positionals = args.parse_flags(|flag| match flag {
            'P' => {
                physical = true;
                Ok(true)
            }
            'L' => {
                physical = false;
                Ok(true)
            }
            _ => Err(format!("-{flag}: invalid option")),
        })?;

        if !positionals.is_empty() {
            return Err(String::from("too many arguments"));
        }

        Ok(Self { physical })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut cwd: Cow<'_, Path> = context.shell.working_dir().into();

        let should_canonicalize = self.physical
            || context
                .shell
                .options()
                .do_not_resolve_symlinks_when_changing_dir;

        if should_canonicalize {
            cwd = cwd.canonicalize()?.into();
        }

        writeln!(
            context.stdout(),
            "{}",
            sys::fs::DisplayPath(cwd.into_owned())
        )?;

        Ok(ExecutionResult::success())
    }
}
