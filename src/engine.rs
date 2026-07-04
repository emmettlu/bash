//! Shell engine implementation. Implements the shell abstraction, interpreter, and supporting
//! runtime facilities.

pub mod arithmetic;
mod braceexpansion;
pub mod builtins;
pub(crate) mod cache;
pub mod callstack;
pub mod commands;
pub mod completion;
pub mod env;
pub mod error;
pub mod escape;
pub mod expansion;
mod extendedtests;
pub mod extensions;
pub mod functions;
pub mod history;
pub mod int_utils;
pub mod interfaces;
mod interp;
mod ioutils;
pub mod jobs;
mod keywords;
pub mod namedoptions;
pub mod openfiles;
pub mod options;
pub mod pathcache;
pub mod pathsearch;
pub mod patterns;
pub mod processes;
mod prompt;
mod regex;
pub mod results;
mod shell;
pub mod sourceinfo;
pub mod sys;
pub mod terminal;
pub mod tests;
pub mod timing;
pub mod trace_categories;
pub mod traps;
pub mod variables;
mod wellknownvars;

/// Re-export parser types used in engine definitions.
pub mod parser {
    pub use crate::parser::{
        BindingParseError, ParseError, SourcePosition, SourcePositionOffset, SourceSpan,
        TestCommandParseError, WordParseError, ast,
    };
}

pub use commands::{CommandArg, ExecutionContext};
pub use error::{BuiltinError, Error, ErrorKind};
pub use extensions::ErrorFormatter;
pub use interp::{ExecutionParameters, ProcessGroupPolicy};
pub use parser::{SourcePosition, SourcePositionOffset, SourceSpan};
pub use results::{ExecutionControlFlow, ExecutionResult, ExecutionSpawnResult, exit_code};
pub(crate) use shell::CreateOptions;
pub use shell::{
    ProfileLoadBehavior, RcLoadBehavior, Shell, ShellBuilder, ShellBuilderState, ShellFd,
};
pub use sourceinfo::SourceInfo;
pub use variables::{ShellValue, ShellVariable};
