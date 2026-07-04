use std::io::Write;

use crate::engine::{ExecutionResult, builtins, escape};

/// Echo text to standard output.
pub(crate) struct EchoCommand {
    /// Suppress the trailing newline from the output.
    no_trailing_newline: bool,

    /// Interpret backslash escapes in the provided text.
    interpret_backslash_escapes: bool,

    /// Do not interpret backslash escapes in the provided text.
    no_interpret_backslash_escapes: bool,

    /// Tokens to echo to standard output.
    args: Vec<String>,
}

impl builtins::Command for EchoCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let (option_args, double_dash_rest) = builtins::split_at_double_dash(args);
        let args = builtins::BuiltinArgs::new(option_args).rest();
        let mut command = Self {
            no_trailing_newline: false,
            interpret_backslash_escapes: false,
            no_interpret_backslash_escapes: false,
            args: Vec::new(),
        };

        let mut first_arg = args.len();
        for (index, arg) in args.iter().enumerate() {
            let Some(flags) = arg.strip_prefix('-') else {
                first_arg = index;
                break;
            };
            if flags.is_empty() || !flags.chars().all(|flag| matches!(flag, 'n' | 'e' | 'E')) {
                first_arg = index;
                break;
            }

            for flag in flags.chars() {
                match flag {
                    'n' => command.no_trailing_newline = true,
                    'e' => {
                        command.interpret_backslash_escapes = true;
                        command.no_interpret_backslash_escapes = false;
                    }
                    'E' => {
                        command.interpret_backslash_escapes = false;
                        command.no_interpret_backslash_escapes = true;
                    }
                    _ => unreachable!(),
                }
            }
        }

        command.args.extend(args[first_arg..].iter().cloned());
        if let Some(rest) = double_dash_rest {
            command.args.extend(rest);
        }

        Ok(command)
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<crate::engine::ExecutionResult, Self::Error> {
        let mut trailing_newline = !self.no_trailing_newline;
        let mut stdout = context.stdout();

        if self.interpret_backslash_escapes && !self.no_interpret_backslash_escapes {
            let mut s = String::new();
            for (i, arg) in self.args.iter().enumerate() {
                if i > 0 {
                    s.push(' ');
                }

                let (expanded_arg, keep_going) = escape::expand_backslash_escapes(
                    arg.as_str(),
                    escape::EscapeExpansionMode::EchoBuiltin,
                )?;
                s.push_str(&String::from_utf8_lossy(expanded_arg.as_slice()));

                if !keep_going {
                    trailing_newline = false;
                    break;
                }
            }

            write!(stdout, "{s}")?;
        } else {
            for (i, arg) in self.args.iter().enumerate() {
                if i > 0 {
                    write!(stdout, " ")?;
                }
                write!(stdout, "{arg}")?;
            }
        }

        if trailing_newline {
            writeln!(stdout)?;
        }

        stdout.flush()?;

        Ok(ExecutionResult::success())
    }
}
