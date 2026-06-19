use crate::core::{ErrorKind, ExecutionResult, builtins};
use clap::Parser;
use std::io::Write;

/// Manage the process umask.
#[derive(Parser)]
pub(crate) struct UmaskCommand {
    /// If MODE is omitted, output in a form that may be reused as input.
    #[arg(short = 'p')]
    print_roundtrippable: bool,

    /// Makes the output symbolic; otherwise an octal number is given.
    #[arg(short = 'S')]
    symbolic_output: bool,

    /// Mode mask.
    mode: Option<String>,
}

impl builtins::Command for UmaskCommand {
    type Error = crate::core::Error;

    async fn execute<SE: crate::core::ShellExtensions>(
        &self,
        context: crate::core::ExecutionContext<'_, SE>,
    ) -> Result<crate::core::ExecutionResult, Self::Error> {
        if let Some(mode) = &self.mode {
            if mode.starts_with(|c: char| c.is_digit(8)) {
                let parsed = crate::core::int_utils::parse(mode.as_str(), 8)?;
                set_umask(parsed)?;
            } else {
                return crate::core::error::unimp("umask setting mode from symbolic value");
            }
        } else {
            let umask = get_umask();

            let formatted = if self.symbolic_output {
                let u = symbolic_mask_from_bits((!umask & 0o700) >> 6);
                let g = symbolic_mask_from_bits((!umask & 0o070) >> 3);
                let o = symbolic_mask_from_bits(!umask & 0o007);
                std::format!("u={u},g={g},o={o}")
            } else {
                std::format!("{umask:04o}")
            };

            if self.print_roundtrippable {
                writeln!(context.stdout(), "umask {formatted}")?;
            } else {
                writeln!(context.stdout(), "{formatted}")?;
            }
        }

        Ok(ExecutionResult::success())
    }
}

const fn get_umask() -> u32 {
    0
}

fn set_umask(value: u32) -> Result<(), crate::core::Error> {
    if value > 0o777 {
        return Err(ErrorKind::InvalidUmask.into());
    }

    // Windows has no process umask; accept valid octal masks as a no-op.
    Ok(())
}

fn symbolic_mask_from_bits(bits: u32) -> String {
    let mut result = String::new();

    if (bits & 0b100) != 0 {
        result.push('r');
    }
    if (bits & 0b010) != 0 {
        result.push('w');
    }
    if (bits & 0b001) != 0 {
        result.push('x');
    }

    result
}
