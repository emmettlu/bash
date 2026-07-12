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
    use crate::parser::ast;

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

    #[test]
    fn test_eof_parse_error_preserves_expected_tokens() {
        let err = parse("echo hello &&").unwrap_err();

        let ParseError::ParsingAtEndOfInputWithExpected { position, expected } = &err else {
            panic!("expected end-of-input PEG details, got {err:?}");
        };

        assert_eq!(position.line, 1);
        assert_eq!(position.column, 14);
        assert!(expected.tokens().next().is_some());
    }

    #[test]
    fn test_io_number_overflow_is_a_parse_error() {
        assert!(parse("echo 999999999999999999999999>output").is_err());
    }

    #[test]
    fn deeply_nested_subshells_return_a_structured_error() {
        let depth = crate::parser::nesting::MAX_NESTING_DEPTH + 1;
        let input = std::format!("{}true{}", "(".repeat(depth), ")".repeat(depth));
        let error = parse(&input).unwrap_err();

        assert!(matches!(error, ParseError::NestingLimitExceeded { .. }));
    }

    #[test]
    fn reserved_words_used_as_arguments_do_not_count_as_nesting() {
        let input = std::format!(
            "echo {}",
            std::iter::repeat_n("if", crate::parser::nesting::MAX_NESTING_DEPTH + 1)
                .collect::<Vec<_>>()
                .join(" ")
        );

        assert!(parse(&input).is_ok());
    }

    #[test]
    fn arithmetic_source_reconstruction_preserves_space_count() {
        let program = parse("((1  +   2))").unwrap();
        let ast::Command::Compound(ast::CompoundCommand::Arithmetic(command), _) =
            &program.complete_commands[0].0[0].0.first.seq[0]
        else {
            panic!("expected arithmetic command");
        };

        assert_eq!(command.expr.value, "1  +   2");
    }
}
