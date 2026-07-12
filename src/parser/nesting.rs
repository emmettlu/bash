//! 递归 parser 使用的统一嵌套限制.

use crate::parser::{SourcePosition, SourceSpan, Token};

pub(crate) const MAX_NESTING_DEPTH: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NestingLimitError {
    pub(crate) depth: usize,
    pub(crate) index: usize,
}

pub(crate) fn check_delimiters(input: &str) -> Result<(), NestingLimitError> {
    let mut stack = Vec::new();
    let mut quote = None;
    let mut escaped = false;

    for (index, c) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
                continue;
            }
            Some('"') => match c {
                '"' => quote = None,
                '\\' => escaped = true,
                _ => {}
            },
            Some(_) => unreachable!("only shell quote characters are tracked"),
            None => match c {
                '\'' | '"' => quote = Some(c),
                '\\' => escaped = true,
                '(' => push_delimiter(&mut stack, ')', index)?,
                '[' => push_delimiter(&mut stack, ']', index)?,
                '{' => push_delimiter(&mut stack, '}', index)?,
                ')' | ']' | '}' if stack.last().is_some_and(|expected| *expected == c) => {
                    stack.pop();
                }
                _ => {}
            },
        }
    }

    Ok(())
}

fn push_delimiter(
    stack: &mut Vec<char>,
    closing: char,
    index: usize,
) -> Result<(), NestingLimitError> {
    stack.push(closing);
    if stack.len() > MAX_NESTING_DEPTH {
        Err(NestingLimitError {
            depth: stack.len(),
            index,
        })
    } else {
        Ok(())
    }
}

pub(crate) fn check_expansions(input: &str) -> Result<(), NestingLimitError> {
    check_expansions_for(input, ExpansionContext::Word)
}

pub(crate) fn check_heredoc_expansions(input: &str) -> Result<(), NestingLimitError> {
    check_expansions_for(input, ExpansionContext::Heredoc)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ExpansionContext {
    Word,
    Heredoc,
}

fn check_expansions_for(input: &str, context: ExpansionContext) -> Result<(), NestingLimitError> {
    let mut stack = Vec::new();
    let mut chars = input.char_indices().peekable();
    let mut quote = None;
    let mut escaped = false;

    while let Some((index, c)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }

        if stack.last() == Some(&'`') {
            match c {
                '`' => {
                    stack.pop();
                }
                '\\' => escaped = true,
                _ => {}
            }
            continue;
        }

        if context == ExpansionContext::Word {
            match quote {
                Some('\'') => {
                    if c == '\'' {
                        quote = None;
                    }
                    continue;
                }
                Some('"') => match c {
                    '"' => {
                        quote = None;
                        continue;
                    }
                    '\\' => {
                        escaped = true;
                        continue;
                    }
                    _ => {}
                },
                Some(_) => unreachable!("only shell quote characters are tracked"),
                None => match c {
                    '\'' | '"' => {
                        quote = Some(c);
                        continue;
                    }
                    '\\' => {
                        escaped = true;
                        continue;
                    }
                    _ => {}
                },
            }
        } else if c == '\\' {
            escaped = true;
            continue;
        }

        if c == '$'
            && let Some((_, opener @ ('(' | '[' | '{'))) = chars.peek().copied()
        {
            let closing = match opener {
                '(' => ')',
                '[' => ']',
                '{' => '}',
                _ => unreachable!("matched expansion delimiters are exhaustive"),
            };
            push_delimiter(&mut stack, closing, index)?;
        } else if context == ExpansionContext::Word
            && matches!(c, '@' | '!' | '?' | '+' | '*')
            && chars.peek().is_some_and(|(_, next)| *next == '(')
        {
            push_delimiter(&mut stack, ')', index)?;
        } else if c == '`' {
            push_delimiter(&mut stack, '`', index)?;
        } else if stack.last().is_some_and(|expected| *expected == c) {
            stack.pop();
        }
    }

    Ok(())
}

pub(crate) fn check_extglobs(input: &str) -> Result<(), NestingLimitError> {
    let mut stack = Vec::new();
    let mut chars = input.char_indices().peekable();
    let mut escaped = false;

    while let Some((index, c)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
        } else if matches!(c, '@' | '!' | '?' | '+' | '*')
            && chars.peek().is_some_and(|(_, next)| *next == '(')
        {
            push_delimiter(&mut stack, ')', index)?;
        } else if c == ')' && stack.last() == Some(&')') {
            stack.pop();
        }
    }

    Ok(())
}

pub(crate) fn check_tokens(tokens: &[Token]) -> Result<(), (SourcePosition, usize)> {
    let mut stack: Vec<&str> = Vec::new();
    let mut at_command_start = true;

    for token in tokens {
        let value = token.to_str();
        let closing = match token {
            Token::Operator(_, _) if value == "(" => Some(")"),
            Token::Word(_, _) if at_command_start && value == "[[" => Some("]]"),
            Token::Word(_, _) if at_command_start && value == "{" => Some("}"),
            Token::Word(_, _) if at_command_start && value == "if" => Some("fi"),
            Token::Word(_, _) if at_command_start && value == "case" => Some("esac"),
            Token::Word(_, _)
                if at_command_start && matches!(value, "for" | "select" | "while" | "until") =>
            {
                Some("done")
            }
            _ => None,
        };

        if let Some(closing) = closing {
            stack.push(closing);
            if stack.len() > MAX_NESTING_DEPTH {
                return Err((token.location().start, stack.len()));
            }
        } else if stack.last().is_some_and(|expected| *expected == value) {
            stack.pop();
        }

        at_command_start = closing.is_some()
            || match token {
                Token::Operator(_, _) => {
                    matches!(value, "(" | ";" | "\n" | "&&" | "||" | "|" | "|&")
                }
                Token::Word(_, _) => {
                    matches!(value, "!" | "time" | "then" | "do" | "else" | "elif")
                }
            };
    }

    Ok(())
}

pub(crate) fn span_at(input: &str, index: usize) -> SourceSpan {
    let prefix = &input[..index];
    let line = prefix.chars().filter(|c| *c == '\n').count() + 1;
    let column = prefix.rsplit_once('\n').map_or_else(
        || prefix.chars().count() + 1,
        |(_, line)| line.chars().count() + 1,
    );
    let start = SourcePosition {
        index: prefix.chars().count(),
        line,
        column,
    };

    SourceSpan { start, end: start }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_delimiters_do_not_count_toward_limit() {
        let single_quoted = std::iter::repeat_n("'('", MAX_NESTING_DEPTH + 1).collect::<String>();
        let double_quoted = std::format!("\"{}\"", "(".repeat(MAX_NESTING_DEPTH + 1));
        assert_eq!(check_delimiters(&single_quoted), Ok(()));
        assert_eq!(check_delimiters(&double_quoted), Ok(()));
    }

    #[test]
    fn expansions_inside_double_quotes_are_limited() {
        let input = std::format!(
            "\"{}x{}\"",
            "$(".repeat(MAX_NESTING_DEPTH + 1),
            ")".repeat(MAX_NESTING_DEPTH + 1),
        );
        assert!(check_expansions(&input).is_err());
    }

    #[test]
    fn heredoc_expansion_check_ignores_literal_syntax() {
        let input = std::format!(
            "{}{}{}{}${{value}}",
            "{".repeat(MAX_NESTING_DEPTH + 1),
            "[".repeat(MAX_NESTING_DEPTH + 1),
            "(".repeat(MAX_NESTING_DEPTH + 1),
            "!(".repeat(MAX_NESTING_DEPTH + 1),
        );

        assert_eq!(check_heredoc_expansions(&input), Ok(()));
    }

    #[test]
    fn heredoc_expansion_check_limits_real_expansions() {
        let input = std::format!(
            "{}x{}",
            "$(".repeat(MAX_NESTING_DEPTH + 1),
            ")".repeat(MAX_NESTING_DEPTH + 1),
        );

        assert!(check_heredoc_expansions(&input).is_err());
    }

    #[test]
    fn deeply_nested_delimiters_are_rejected() {
        let input = "(".repeat(MAX_NESTING_DEPTH + 1);
        let error = check_delimiters(&input).unwrap_err();
        assert_eq!(error.depth, MAX_NESTING_DEPTH + 1);
    }
}
