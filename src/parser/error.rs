use crate::parser::tokenizer;

/// Represents an error that occurred while parsing tokens.
#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    /// A parsing error occurred near the given position.
    #[error("syntax error at line {} col {}", .0.line, .0.column)]
    ParsingNear(crate::parser::SourcePosition),

    /// A parsing error occurred near the given position, with PEG expected-token details.
    #[error("syntax error at line {} col {}: expected {}", .position.line, .position.column, .expected)]
    ParsingNearWithExpected {
        /// The source position near which parsing failed.
        position: crate::parser::SourcePosition,
        /// The PEG expected-token set reported at that position.
        expected: peg::error::ExpectedSet,
    },

    /// A parsing error occurred at the end of the input.
    #[error("syntax error at end of input")]
    ParsingAtEndOfInput,

    /// 输入末尾发生解析错误, 并保留 PEG 期望 token 详情.
    #[error("syntax error at end of input at line {} col {}: expected {}", .position.line, .position.column, .expected)]
    ParsingAtEndOfInputWithExpected {
        /// 输入末尾的源码位置.
        position: crate::parser::SourcePosition,
        /// PEG 在输入末尾报告的期望 token 集合.
        expected: peg::error::ExpectedSet,
    },

    /// An error occurred while tokenizing the input stream.
    #[error("{} (detected near line {} col {})", .inner, .position.line, .position.column)]
    Tokenizing {
        /// The inner error.
        inner: tokenizer::TokenizerError,
        /// The position of the error.
        position: crate::parser::SourcePosition,
    },

    /// Parser 嵌套深度超过上限.
    #[error("parser nesting limit {limit} exceeded at line {} col {}", .position.line, .position.column)]
    NestingLimitExceeded {
        /// 支持的最大嵌套深度.
        limit: usize,
        /// 超过上限的位置.
        position: crate::parser::SourcePosition,
    },
}

/// Represents a parsing error with its location information
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ParseErrorLocation {
    #[from]
    inner: peg::error::ParseError<peg::str::LineCol>,
}

/// Represents an error that occurred while parsing a word.
#[derive(Debug, thiserror::Error)]
pub enum WordParseError {
    /// An error occurred while parsing an arithmetic expression.
    #[error("failed to parse arithmetic expression")]
    ArithmeticExpression(ParseErrorLocation),

    /// An error occurred while parsing a shell pattern.
    #[error("failed to parse pattern")]
    Pattern(ParseErrorLocation),

    /// An error occurred while parsing a prompt string.
    #[error("failed to parse prompt string")]
    Prompt(ParseErrorLocation),

    /// An error occurred while parsing a parameter.
    #[error("failed to parse parameter '{0}'")]
    Parameter(String, ParseErrorLocation),

    /// An error occurred while parsing for brace expansion.
    #[error("failed to parse for brace expansion: '{0}'")]
    BraceExpansion(String, ParseErrorLocation),

    /// An error occurred while parsing a word.
    #[error("failed to parse word '{0}'")]
    Word(String, ParseErrorLocation),

    /// Parser 嵌套深度超过上限.
    #[error("parser nesting limit {limit} exceeded at line {} col {}", .position.line, .position.column)]
    NestingLimitExceeded {
        /// 支持的最大嵌套深度.
        limit: usize,
        /// 超过上限的位置.
        position: crate::parser::SourcePosition,
    },
}

/// 表示解析非扩展 test 命令时发生的错误.
#[derive(Debug, thiserror::Error)]
pub enum TestCommandParseError {
    /// Test 表达式不符合语法.
    #[error(transparent)]
    Parsing(#[from] peg::error::ParseError<usize>),
    /// Parser 嵌套深度超过上限.
    #[error("parser nesting limit {limit} exceeded")]
    NestingLimitExceeded {
        /// 支持的最大嵌套深度.
        limit: usize,
    },
}

/// Represents an error that occurred while parsing a key-binding specification.
#[derive(Debug, thiserror::Error)]
pub enum BindingParseError {
    /// An unknown error occurred while parsing a key-binding specification.
    #[error("unknown error while parsing key-binding: '{0}'")]
    Unknown(String),

    /// A key code was missing from the key-binding specification.
    #[error("missing key code in key-binding")]
    MissingKeyCode,
}

pub(crate) fn convert_peg_parse_error(
    err: &peg::error::ParseError<usize>,
    tokens: &[crate::parser::Token],
) -> ParseError {
    let approx_token_index = err.location;

    if approx_token_index < tokens.len() {
        let token = &tokens[approx_token_index];
        let position = token.location().start;

        if err.expected.tokens().next().is_some() {
            ParseError::ParsingNearWithExpected {
                position,
                expected: err.expected.clone(),
            }
        } else {
            ParseError::ParsingNear(position)
        }
    } else if err.expected.tokens().next().is_some() {
        let position = tokens
            .last()
            .map_or_else(crate::parser::SourcePosition::default, |token| {
                token.location().end
            });
        ParseError::ParsingAtEndOfInputWithExpected {
            position,
            expected: err.expected.clone(),
        }
    } else {
        ParseError::ParsingAtEndOfInput
    }
}
