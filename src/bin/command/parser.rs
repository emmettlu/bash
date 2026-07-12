use std::collections::VecDeque;
use std::fmt::{Display, Formatter};

use super::lexer::{self, Operator, Token, TokenKind};
use super::{
    AndOrChain, LogicalOperator, Pipeline, Program, Redirect, RedirectKind, SimpleCommand,
};

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub position: usize,
    pub incomplete: bool,
}

impl Display for ParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "第 {} 个字符附近: {}",
            self.position + 1,
            self.message
        )
    }
}

pub(super) fn parse(input: &str) -> Result<Program, ParseError> {
    let tokens = lexer::lex(input).map_err(|error| ParseError {
        message: error.message,
        position: error.position,
        incomplete: error.incomplete,
    })?;
    Parser {
        tokens: tokens.into(),
        last_position: 0,
    }
    .program()
}

struct Parser {
    tokens: VecDeque<Token>,
    last_position: usize,
}

impl Parser {
    fn program(mut self) -> Result<Program, ParseError> {
        let mut chains = Vec::new();
        self.skip_sequences();
        while !self.tokens.is_empty() {
            chains.push(self.and_or_chain()?);
            if self.tokens.is_empty() {
                break;
            }
            if !self.consume_operator(Operator::Sequence) {
                return Err(self.error("预期使用 ';' 或换行分隔命令", false));
            }
            self.skip_sequences();
        }
        Ok(Program { chains })
    }

    fn and_or_chain(&mut self) -> Result<AndOrChain, ParseError> {
        let first = self.pipeline()?;
        let mut rest = Vec::new();
        loop {
            let operator = if self.consume_operator(Operator::And) {
                Some(LogicalOperator::And)
            } else if self.consume_operator(Operator::Or) {
                Some(LogicalOperator::Or)
            } else {
                None
            };
            let Some(operator) = operator else {
                break;
            };
            self.skip_sequences();
            if self.tokens.is_empty() {
                return Err(self.error("逻辑运算符后缺少命令", true));
            }
            rest.push((operator, self.pipeline()?));
        }
        Ok(AndOrChain { first, rest })
    }

    fn pipeline(&mut self) -> Result<Pipeline, ParseError> {
        let mut commands = vec![self.simple_command()?];
        while self.consume_operator(Operator::Pipe) {
            self.skip_sequences();
            if self.tokens.is_empty() {
                return Err(self.error("管道符后缺少命令", true));
            }
            commands.push(self.simple_command()?);
        }
        Ok(Pipeline { commands })
    }

    fn simple_command(&mut self) -> Result<SimpleCommand, ParseError> {
        let mut words = Vec::new();
        let mut redirects = Vec::new();

        while let Some(token) = self.tokens.front() {
            match &token.kind {
                TokenKind::Word(_) => {
                    let TokenKind::Word(word) = self.take().kind else {
                        unreachable!()
                    };
                    words.push(word);
                }
                TokenKind::Operator(
                    operator @ (Operator::Input | Operator::Output | Operator::Append),
                ) => {
                    let kind = match operator {
                        Operator::Input => RedirectKind::Input,
                        Operator::Output => RedirectKind::Output,
                        Operator::Append => RedirectKind::Append,
                        _ => unreachable!(),
                    };
                    let position = token.position;
                    self.take();
                    let Some(Token {
                        kind: TokenKind::Word(_),
                        ..
                    }) = self.tokens.front()
                    else {
                        return Err(ParseError {
                            message: "重定向后缺少文件名".into(),
                            position,
                            incomplete: self.tokens.is_empty(),
                        });
                    };
                    let TokenKind::Word(target) = self.take().kind else {
                        unreachable!()
                    };
                    redirects.push(Redirect { kind, target });
                }
                TokenKind::Operator(_) => break,
            }
        }

        if words.is_empty() && redirects.is_empty() {
            return Err(self.error("预期命令", self.tokens.is_empty()));
        }
        Ok(SimpleCommand { words, redirects })
    }

    fn take(&mut self) -> Token {
        let token = self.tokens.pop_front().expect("token should exist");
        self.last_position = token.position;
        token
    }

    fn consume_operator(&mut self, expected: Operator) -> bool {
        if matches!(
            self.tokens.front(),
            Some(Token {
                kind: TokenKind::Operator(actual),
                ..
            }) if *actual == expected
        ) {
            self.take();
            true
        } else {
            false
        }
    }

    fn skip_sequences(&mut self) {
        while self.consume_operator(Operator::Sequence) {}
    }

    fn error(&self, message: &str, incomplete: bool) -> ParseError {
        ParseError {
            message: message.into(),
            position: self
                .tokens
                .front()
                .map_or(self.last_position, |token| token.position),
            incomplete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pipeline_and_logic() {
        let program = parse("echo one | findstr one && echo ok").unwrap();
        assert_eq!(program.chains.len(), 1);
        assert_eq!(program.chains[0].first.commands.len(), 2);
        assert_eq!(program.chains[0].rest.len(), 1);
    }
}
