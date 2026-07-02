//! Library implementing interactive command input and completion for the shell.

mod error;
pub use error::ShellError;

mod interactive_shell;
pub use interactive_shell::{InteractiveExecutionResult, InteractiveOptions, InteractiveShell};

mod input_backend;
pub use input_backend::{InputBackend, InteractivePrompt, ReadResult};

mod options;
pub use options::UIOptions;

mod refs;
pub use refs::ShellRef;

mod term_detection;
mod term_integration;
mod trace_categories;
pub mod win_term;

pub mod highlighting;

mod completion;

mod basic;
pub use basic::BasicInputBackend;

mod minimal;
pub use minimal::MinimalInputBackend;
