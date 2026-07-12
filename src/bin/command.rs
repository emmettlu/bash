//! 最小 Shell 的命令模型、解析器和执行器.

mod builtin;
mod lexer;
mod parser;
mod runtime;

pub use runtime::Shell;

use std::fmt::{Display, Formatter};

/// 一段已解析的 Shell 程序.
#[derive(Debug)]
pub struct Program {
    chains: Vec<AndOrChain>,
}

#[derive(Debug)]
struct AndOrChain {
    first: Pipeline,
    rest: Vec<(LogicalOperator, Pipeline)>,
}

#[derive(Clone, Copy, Debug)]
enum LogicalOperator {
    And,
    Or,
}

#[derive(Debug)]
struct Pipeline {
    commands: Vec<SimpleCommand>,
}

#[derive(Debug)]
struct SimpleCommand {
    words: Vec<Word>,
    redirects: Vec<Redirect>,
}

#[derive(Debug)]
struct Redirect {
    kind: RedirectKind,
    target: Word,
}

#[derive(Clone, Copy, Debug)]
enum RedirectKind {
    Input,
    Output,
    Append,
}

/// 一个保留引用语义的 Shell word.
#[derive(Clone, Debug, Default)]
struct Word {
    segments: Vec<WordSegment>,
}

#[derive(Clone, Debug)]
struct WordSegment {
    text: String,
    expand: bool,
}

impl Word {
    fn push(&mut self, text: char, expand: bool) {
        if let Some(segment) = self.segments.last_mut()
            && segment.expand == expand
        {
            segment.text.push(text);
            return;
        }
        self.segments.push(WordSegment {
            text: text.to_string(),
            expand,
        });
    }

    fn push_empty(&mut self, expand: bool) {
        if self.segments.is_empty() {
            self.segments.push(WordSegment {
                text: String::new(),
                expand,
            });
        }
    }
}

/// `besh` 的解析或执行错误.
#[derive(Debug)]
pub enum Error {
    Parse(parser::ParseError),
    Io(std::io::Error),
    Message(String),
}

impl Error {
    /// 返回错误是否表示输入尚未完成.
    pub const fn is_incomplete(&self) -> bool {
        matches!(self, Self::Parse(error) if error.incomplete)
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => Display::fmt(error, formatter),
            Self::Io(error) => Display::fmt(error, formatter),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<parser::ParseError> for Error {
    fn from(error: parser::ParseError) -> Self {
        Self::Parse(error)
    }
}

/// 解析一段 Shell 输入.
pub fn parse(input: &str) -> Result<Program, Error> {
    parser::parse(input).map_err(Into::into)
}
