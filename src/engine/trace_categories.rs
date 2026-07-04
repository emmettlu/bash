//! Trace utilities

/// Trace category for arithmetic expression parsing and evaluation.
pub const ARITHMETIC: &str = "arithmetic";
/// Trace category for command execution.
pub const COMMANDS: &str = "commands";
/// Trace category for completion.
pub const COMPLETION: &str = "completion";
/// Trace category for word expansion.
pub const EXPANSION: &str = "expansion";
/// Trace category for function calls.
pub const FUNCTIONS: &str = "functions";
/// Trace category for user input.
pub const INPUT: &str = "input";
/// Trace category for job control.
pub const JOBS: &str = "jobs";
/// Trace category for parsing.
pub const PARSE: &str = "parse";
/// Trace category for shell patterns.
pub const PATTERN: &str = "pattern";
/// Trace category for tokenization.
pub const TOKENIZE: &str = "tokenize";
/// Trace category for unimplemented behavior.
pub const UNIMPLEMENTED: &str = "unimplemented";
