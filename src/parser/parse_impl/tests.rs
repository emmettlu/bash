//! Parser snapshot test harness.

mod and_or_lists;
mod assignments;
mod complex;
mod compound_commands;
mod extended_test;
mod functions;
mod here_docs;
mod pipelines;
mod redirections;
mod simple_commands;

use crate::parser::ast::Program;
use crate::parser::error::ParseError;
use crate::parser::parse_impl::{Parser, ParserOptions};
use anyhow::Result;

/// Wrapper struct for serializing parse results with input context.
#[derive(serde::Serialize)]
pub struct ParseResult<'a, T> {
    pub input: &'a str,
    pub result: &'a T,
}

/// Macro to assert snapshots with location information redacted.
/// This makes snapshots stable across parser changes that only affect source locations.
#[macro_export]
macro_rules! assert_snapshot_redacted {
    ($value:expr) => {{
        let mut settings = insta::Settings::clone_current();
        settings.add_redaction(".**.loc", "[location]");
        settings.bind(|| {
            insta::assert_ron_snapshot!($value);
        });
    }};
}

/// 使用唯一 parser 解析输入.
pub fn parse(input: &str) -> Result<Program, ParseError> {
    let options = ParserOptions::default();
    let mut parser = Parser::new(std::io::Cursor::new(input), &options);
    parser.parse_program()
}

/// 使用标准 parser 运行测试并创建快照.
pub fn test_with_snapshot(input: &str) -> Result<Program> {
    parse(input).map_err(|e| anyhow::anyhow!("parser failed: {e}\nInput: {input}"))
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let result = parse("echo hello");
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_error_preserves_expected_tokens() {
        let err = parse("echo hello && ;").unwrap_err();

        let ParseError::ParsingNearWithExpected { position, expected } = &err else {
            panic!("expected PEG expected-token details, got {err:?}");
        };

        assert_eq!(position.line, 1);
        assert_eq!(position.column, 15);
        assert!(expected.tokens().next().is_some());
    }
}
