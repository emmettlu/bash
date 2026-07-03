use crate::engine::sys;
use clap::Parser;
use std::io::Write;

use crate::engine::{ExecutionResult, builtins};

#[derive(Debug, thiserror::Error)]
pub(crate) enum DirError {
    /// Directory stack is empty.
    #[error("directory stack is empty")]
    DirStackEmpty,

    /// A shell error occurred.
    #[error(transparent)]
    ShellError(#[from] crate::engine::Error),
}

impl From<&DirError> for u8 {
    fn from(value: &DirError) -> Self {
        match value {
            DirError::DirStackEmpty => crate::engine::exit_code::GENERAL_ERROR,
            DirError::ShellError(e) => u8::from(e),
        }
    }
}

impl crate::engine::BuiltinError for DirError {}

/// Manage the current directory stack.
#[derive(Default, Parser)]
pub(crate) struct DirsCommand {
    /// Clear the directory stack.
    #[arg(short = 'c')]
    clear: bool,

    /// Don't tilde-shorten paths.
    #[arg(short = 'l')]
    tilde_long: bool,

    /// Print one directory per line instead of all on one line.
    #[arg(short = 'p')]
    print_one_per_line: bool,

    /// Print one directory per line with its index.
    #[arg(short = 'v')]
    print_one_per_line_with_index: bool,
    //
    // TODO(dirs): implement +N and -N
}

impl builtins::Command for DirsCommand {
    type Error = crate::engine::Error;

    async fn execute<SE: crate::engine::ShellExtensions>(
        &self,
        context: crate::engine::ExecutionContext<'_, SE>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        if self.clear {
            context.shell.directory_stack_mut().clear();
        } else {
            let one_per_line = self.print_one_per_line || self.print_one_per_line_with_index;
            let mut stdout = context.stdout();
            let dirs = std::iter::once(context.shell.working_dir()).chain(
                context
                    .shell
                    .directory_stack()
                    .iter()
                    .rev()
                    .map(|p| p.as_path()),
            );

            for (i, dir) in dirs.enumerate() {
                if !one_per_line && i > 0 {
                    write!(stdout, " ")?;
                }

                if self.print_one_per_line_with_index {
                    write!(stdout, "{i:2}  ")?;
                }

                let mut dir_str = sys::fs::display_path(dir);

                if !self.tilde_long {
                    dir_str = context.shell.tilde_shorten(dir_str);
                }

                write!(stdout, "{dir_str}")?;

                if one_per_line {
                    writeln!(stdout)?;
                }
            }

            if !one_per_line {
                writeln!(stdout)?;
            }

            return Ok(ExecutionResult::success());
        }

        Ok(ExecutionResult::success())
    }
}
