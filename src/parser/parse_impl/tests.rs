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
use crate::parser::parse_impl::{Parser, ParserImpl, ParserOptions};
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

/// A named parser configuration for test output clarity.
#[derive(Debug, Clone)]
pub struct ParserConfig {
    pub name: &'static str,
    pub parser_impl: ParserImpl,
}

/// Returns all available parser implementations for testing.
pub fn parser_configs() -> Vec<ParserConfig> {
    vec![ParserConfig {
        name: "peg",
        parser_impl: ParserImpl::Peg,
    }]
}

/// Helper to parse input with a specific parser configuration.
pub fn parse_with_config(input: &str, config: &ParserConfig) -> Result<Program, ParseError> {
    let options = ParserOptions {
        parser_impl: config.parser_impl,
        ..Default::default()
    };

    let mut parser = Parser::new(std::io::Cursor::new(input), &options);
    parser.parse_program()
}

/// Run a test and create snapshot for the canonical parser.
pub fn test_with_snapshot(input: &str) -> Result<Program> {
    let peg_config = ParserConfig {
        name: "peg",
        parser_impl: ParserImpl::Peg,
    };
    parse_with_config(input, &peg_config)
        .map_err(|e| anyhow::anyhow!("parser failed: {e}\nInput: {input}"))
}

#[cfg(test)]
mod harness_tests {
    use super::*;

    #[test]
    fn test_parser_configs_includes_peg() {
        let configs = parser_configs();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "peg");
    }

    #[test]
    fn test_parse_with_config_basic() {
        let config = ParserConfig {
            name: "peg",
            parser_impl: ParserImpl::Peg,
        };
        let result = parse_with_config("echo hello", &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_error_preserves_expected_tokens() {
        let config = ParserConfig {
            name: "peg",
            parser_impl: ParserImpl::Peg,
        };
        let err = parse_with_config("echo hello && ;", &config).unwrap_err();

        let ParseError::ParsingNearWithExpected { position, expected } = &err else {
            panic!("expected PEG expected-token details, got {err:?}");
        };

        assert_eq!(position.line, 1);
        assert_eq!(position.column, 15);
        assert!(expected.tokens().next().is_some());
    }
}
