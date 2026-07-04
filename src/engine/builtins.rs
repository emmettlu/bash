//! Facilities for implementing and managing builtins

pub use futures::future::BoxFuture;
use nanocolor::Colorize as _;
use std::io::Write;

use crate::engine::{BuiltinError, CommandArg, commands, error, results};

/// Type of a function implementing a built-in command.
///
/// # Arguments
///
/// * The context in which the command is being executed.
/// * The arguments to the command.
#[allow(type_alias_bounds)]
pub type CommandExecuteFunc = fn(
    commands::ExecutionContext<'_>,
    Vec<commands::CommandArg>,
) -> BoxFuture<'_, Result<results::ExecutionResult, error::Error>>;

/// Type of a function to retrieve help content for a built-in command.
///
/// # Arguments
///
/// * `name` - The name of the command.
/// * `content_type` - The type of content to retrieve.
/// * `options` - Additional options for content retrieval.
pub type CommandContentFunc =
    fn(&str, ContentType, &ContentOptions) -> Result<String, error::Error>;

/// Trait implemented by built-in shell commands.
pub trait Command: Sized {
    /// The error type returned by the command.
    type Error: BuiltinError + 'static;

    /// Instantiates the built-in command with the given arguments.
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments to the command.
    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>;

    /// Executes the built-in command in the provided context.
    ///
    /// # Arguments
    ///
    /// * `context` - The context in which the command is being executed.
    // NOTE: we use desugared async here because we need a Send marker
    fn execute(
        &self,
        context: commands::ExecutionContext<'_>,
    ) -> impl std::future::Future<Output = Result<results::ExecutionResult, Self::Error>>
    + std::marker::Send;

    /// Returns the textual help content associated with the command.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the command.
    /// * `content_type` - The type of content to retrieve.
    /// * `options` - Additional options for content retrieval.
    fn get_content(
        name: &str,
        content_type: ContentType,
        options: &ContentOptions,
    ) -> Result<String, error::Error> {
        let description = "shell builtin";
        let name_display = if options.colorized {
            format!("{}", nanocolor::style(name).cyan().bold())
        } else {
            name.to_owned()
        };

        match content_type {
            ContentType::DetailedHelp => Ok(format!("{name_display}: {description}\n")),
            ContentType::ShortUsage => Ok(format!("{name}: {name}\n")),
            ContentType::ShortDescription => Ok(format!("{name} - {description}\n")),
        }
    }
}

/// Trait implemented by built-in shell commands that take specially handled declarations
/// as arguments.
pub trait DeclarationCommand: Command {
    /// Stores the declarations within the command instance.
    ///
    /// # Arguments
    ///
    /// * `declarations` - The declarations to store.
    fn set_declarations(&mut self, declarations: Vec<commands::CommandArg>);
}

/// Type of help content, typically associated with a built-in command.
pub enum ContentType {
    /// Detailed help content for the command.
    DetailedHelp,
    /// Short usage information for the command.
    ShortUsage,
    /// Short description for the command.
    ShortDescription,
}

/// Options for retrieving built-in command content.
#[derive(Default)]
pub struct ContentOptions {
    /// Whether or not the content should be colorized.
    pub colorized: bool,
}

/// Encapsulates a registration for a built-in command.
#[derive(Clone)]
pub struct Registration {
    /// Function to execute the builtin.
    pub execute_func: CommandExecuteFunc,

    /// Function to retrieve the builtin's content/help text.
    pub content_func: CommandContentFunc,

    /// Has this registration been disabled?
    pub disabled: bool,

    /// Is the builtin classified as "special" by specification?
    pub special_builtin: bool,

    /// Is this builtin one that takes specially handled declarations?
    pub declaration_builtin: bool,
}

impl Registration {
    /// Updates the given registration to mark it for a special builtin.
    #[must_use]
    pub const fn special(self) -> Self {
        Self {
            special_builtin: true,
            ..self
        }
    }
}

/// 在参数列表中找到第一个 `--`, 将其前、自身和之后参数分开返回。
pub fn split_at_double_dash(
    args: impl IntoIterator<Item = String>,
) -> (Vec<String>, Option<Vec<String>>) {
    let mut args: Vec<String> = args.into_iter().collect();
    if let Some(pos) = args.iter().position(|a| a == "--") {
        let rest = args.split_off(pos);
        (args, Some(rest))
    } else {
        (args, None)
    }
}

/// 内置命令参数游标, 用于替代 derive parser 的轻量解析。
pub struct BuiltinArgs {
    args: Vec<String>,
    index: usize,
    stop_options: bool,
}

impl BuiltinArgs {
    pub fn new(args: impl IntoIterator<Item = String>) -> Self {
        let args = args.into_iter().collect::<Vec<_>>();
        let index = usize::from(!args.is_empty());
        Self {
            args,
            index,
            stop_options: false,
        }
    }

    pub fn next_arg(&mut self) -> Option<String> {
        let value = self.args.get(self.index).cloned()?;
        self.index += 1;
        Some(value)
    }

    pub fn peek(&self) -> Option<&str> {
        self.args.get(self.index).map(String::as_str)
    }

    pub fn next_value(&mut self, option: &str) -> Result<String, String> {
        self.next_arg()
            .ok_or_else(|| format!("{option}: option requires an argument"))
    }

    pub fn rest(mut self) -> Vec<String> {
        self.drain_rest()
    }

    fn drain_rest(&mut self) -> Vec<String> {
        self.args.drain(self.index..).collect()
    }

    pub fn parse_flags(
        &mut self,
        mut on_flag: impl FnMut(char) -> Result<bool, String>,
    ) -> Result<Vec<String>, String> {
        let mut positionals = Vec::new();
        while let Some(arg) = self.next_arg() {
            if self.stop_options || arg == "--" {
                if arg == "--" {
                    self.stop_options = true;
                } else {
                    positionals.push(arg);
                }
                positionals.extend(self.drain_rest());
                break;
            }

            let Some(flags) = arg.strip_prefix('-') else {
                positionals.push(arg);
                positionals.extend(self.drain_rest());
                break;
            };

            if flags.is_empty() {
                positionals.push(arg);
                positionals.extend(self.drain_rest());
                break;
            }

            for flag in flags.chars() {
                if !on_flag(flag)? {
                    positionals.extend(self.drain_rest());
                    return Ok(positionals);
                }
            }
        }
        Ok(positionals)
    }
}

/// A simple command that can be registered as a built-in.
pub trait SimpleCommand {
    /// Returns the content of the built-in command.
    fn get_content(
        name: &str,
        content_type: ContentType,
        options: &ContentOptions,
    ) -> Result<String, error::Error>;

    /// Executes the built-in command.
    fn execute<I: Iterator<Item = S>, S: AsRef<str>>(
        context: commands::ExecutionContext<'_>,
        args: I,
    ) -> Result<results::ExecutionResult, error::Error>;
}

/// Returns a built-in command registration, given an implementation of the
/// `SimpleCommand` trait.
pub fn simple_builtin<B: SimpleCommand + Send + Sync>() -> Registration {
    Registration {
        execute_func: exec_simple_builtin::<B>,
        content_func: B::get_content,
        disabled: false,
        special_builtin: false,
        declaration_builtin: false,
    }
}

/// Returns a built-in command registration, given an implementation of the
/// `Command` trait.
pub fn builtin<B: Command + Send + Sync>() -> Registration {
    Registration {
        execute_func: exec_builtin::<B>,
        content_func: get_builtin_content::<B>,
        disabled: false,
        special_builtin: false,
        declaration_builtin: false,
    }
}

/// Returns a built-in command registration, given an implementation of the
/// `DeclarationCommand` trait. Used for select commands that can take parsed
/// declarations as arguments.
pub fn decl_builtin<B: DeclarationCommand + Send + Sync>() -> Registration {
    Registration {
        execute_func: exec_declaration_builtin::<B>,
        content_func: get_builtin_content::<B>,
        disabled: false,
        special_builtin: false,
        declaration_builtin: true,
    }
}

#[allow(clippy::too_long_first_doc_paragraph)]
/// Returns a built-in command registration, given an implementation of the
/// `DeclarationCommand` trait that can be default-constructed. The command
/// implementation is default-constructed. Arguments are passed directly to
/// the command via `set_declarations`. This is primarily only expected to be used with
/// select builtin commands that wrap other builtins (e.g., "builtin").
pub fn raw_arg_builtin<B: DeclarationCommand + Default + Send + Sync>() -> Registration {
    Registration {
        execute_func: exec_raw_arg_builtin::<B>,
        content_func: get_builtin_content::<B>,
        disabled: false,
        special_builtin: false,
        declaration_builtin: true,
    }
}

fn get_builtin_content<T: Command + Send + Sync>(
    name: &str,
    content_type: ContentType,
    options: &ContentOptions,
) -> Result<String, error::Error> {
    T::get_content(name, content_type, options)
}

fn exec_simple_builtin<T: SimpleCommand + Send + Sync>(
    context: commands::ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<results::ExecutionResult, error::Error>> {
    Box::pin(async move {
        let plain_args = args.into_iter().map(CommandArg::into_string);
        T::execute(context, plain_args)
    })
}

fn exec_builtin<T: Command + Send + Sync>(
    context: commands::ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<results::ExecutionResult, error::Error>> {
    Box::pin(async move {
        let plain_args = args.into_iter().map(CommandArg::into_string);

        let result = T::new(plain_args);
        let command = match result {
            Ok(command) => command,
            Err(e) => {
                let _ = writeln!(context.stderr(), "{e}");
                return Ok(results::ExecutionResult::invalid_usage());
            }
        };

        call_builtin(command, context).await
    })
}

fn exec_declaration_builtin<T: DeclarationCommand + Send + Sync>(
    context: commands::ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<results::ExecutionResult, error::Error>> {
    Box::pin(async move {
        let mut options = vec![];
        let mut declarations = vec![];

        for (i, arg) in args.into_iter().enumerate() {
            match arg {
                CommandArg::String(s)
                    if i == 0 || (s.len() > 1 && (s.starts_with('-') || s.starts_with('+'))) =>
                {
                    options.push(s);
                }
                _ => declarations.push(arg),
            }
        }

        let result = T::new(options);
        let mut command = match result {
            Ok(command) => command,
            Err(e) => {
                let _ = writeln!(context.stderr(), "{e}");
                return Ok(results::ExecutionResult::invalid_usage());
            }
        };

        command.set_declarations(declarations);

        call_builtin(command, context).await
    })
}

fn exec_raw_arg_builtin<T: DeclarationCommand + Default + Send + Sync>(
    context: commands::ExecutionContext<'_>,
    args: Vec<CommandArg>,
) -> BoxFuture<'_, Result<results::ExecutionResult, error::Error>> {
    Box::pin(async move {
        let mut command = T::default();
        command.set_declarations(args);
        call_builtin(command, context).await
    })
}

async fn call_builtin(
    command: impl Command,
    context: commands::ExecutionContext<'_>,
) -> Result<results::ExecutionResult, error::Error> {
    let builtin_name = context.command_name.clone();
    let result = command
        .execute(context)
        .await
        .map_err(|e| error::ErrorKind::BuiltinError(Box::new(e), builtin_name))?;

    Ok(result)
}
