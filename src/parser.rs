//! Implements a tokenizer and parsers for bash shell syntax.

pub mod arithmetic;
pub mod ast;
pub mod pattern;
pub mod prompt;
pub mod readline_binding;
pub mod test_command;
pub mod word;

mod cache;
mod error;
mod nesting;
mod parse_impl;
mod source;
mod tokenizer;

#[cfg(test)]
mod snapshot_tests;

pub use error::{
    BindingParseError, ParseError, ParseErrorLocation, TestCommandParseError, WordParseError,
};

pub use parse_impl::{Parser, ParserBuilder, ParserOptions};

pub use source::{SourcePosition, SourcePositionOffset, SourceSpan};
pub use tokenizer::{
    Token, TokenizerError, TokenizerOptions, tokenize_str, tokenize_str_with_options,
    uncached_tokenize_str, unquote_str,
};
