use super::Word;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Operator {
    Sequence,
    Pipe,
    And,
    Or,
    Input,
    Output,
    Append,
}

#[derive(Debug)]
pub(super) enum TokenKind {
    Word(Word),
    Operator(Operator),
}

#[derive(Debug)]
pub(super) struct Token {
    pub kind: TokenKind,
    pub position: usize,
}

#[derive(Debug)]
pub(super) struct LexError {
    pub message: String,
    pub position: usize,
    pub incomplete: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Quote {
    None,
    Single,
    Double,
}

pub(super) fn lex(input: &str) -> Result<Vec<Token>, LexError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut may_start_comment = true;

    while index < chars.len() {
        match chars[index] {
            ' ' | '\t' | '\r' => {
                index += 1;
                may_start_comment = true;
            }
            '\n' | ';' => {
                push_operator(&mut tokens, Operator::Sequence, index);
                index += 1;
                may_start_comment = true;
            }
            '#' if may_start_comment => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            '&' if chars.get(index + 1) == Some(&'&') => {
                push_operator(&mut tokens, Operator::And, index);
                index += 2;
                may_start_comment = true;
            }
            '&' => {
                return Err(LexError {
                    message: "最小实现暂不支持后台运算符 '&'".into(),
                    position: index,
                    incomplete: false,
                });
            }
            '|' if chars.get(index + 1) == Some(&'|') => {
                push_operator(&mut tokens, Operator::Or, index);
                index += 2;
                may_start_comment = true;
            }
            '|' => {
                push_operator(&mut tokens, Operator::Pipe, index);
                index += 1;
                may_start_comment = true;
            }
            '<' => {
                push_operator(&mut tokens, Operator::Input, index);
                index += 1;
                may_start_comment = true;
            }
            '>' if chars.get(index + 1) == Some(&'>') => {
                push_operator(&mut tokens, Operator::Append, index);
                index += 2;
                may_start_comment = true;
            }
            '>' => {
                push_operator(&mut tokens, Operator::Output, index);
                index += 1;
                may_start_comment = true;
            }
            _ => {
                let position = index;
                let (word, next) = lex_word(&chars, index)?;
                tokens.push(Token {
                    kind: TokenKind::Word(word),
                    position,
                });
                index = next;
                may_start_comment = false;
            }
        }
    }

    Ok(tokens)
}

fn push_operator(tokens: &mut Vec<Token>, operator: Operator, position: usize) {
    if operator == Operator::Sequence
        && matches!(
            tokens.last(),
            None | Some(Token {
                kind: TokenKind::Operator(Operator::Sequence),
                ..
            })
        )
    {
        return;
    }
    tokens.push(Token {
        kind: TokenKind::Operator(operator),
        position,
    });
}

fn lex_word(chars: &[char], start: usize) -> Result<(Word, usize), LexError> {
    let mut word = Word::default();
    let mut quote = Quote::None;
    let mut index = start;

    while index < chars.len() {
        let current = chars[index];
        match quote {
            Quote::None => match current {
                ' ' | '\t' | '\r' | '\n' | ';' | '|' | '&' | '<' | '>' => break,
                '\'' => {
                    quote = Quote::Single;
                    word.push_empty(false);
                    index += 1;
                }
                '"' => {
                    quote = Quote::Double;
                    word.push_empty(true);
                    index += 1;
                }
                '\\' => {
                    let Some(next) = chars.get(index + 1).copied() else {
                        return Err(LexError {
                            message: "反斜杠后缺少字符".into(),
                            position: index,
                            incomplete: true,
                        });
                    };
                    word.push(next, false);
                    index += 2;
                }
                _ => {
                    word.push(current, true);
                    index += 1;
                }
            },
            Quote::Single => {
                if current == '\'' {
                    quote = Quote::None;
                } else {
                    word.push(current, false);
                }
                index += 1;
            }
            Quote::Double => match current {
                '"' => {
                    quote = Quote::None;
                    index += 1;
                }
                '\\' if matches!(chars.get(index + 1), Some('$' | '"' | '\\')) => {
                    word.push(chars[index + 1], false);
                    index += 2;
                }
                _ => {
                    word.push(current, true);
                    index += 1;
                }
            },
        }
    }

    if quote != Quote::None {
        return Err(LexError {
            message: "引号未闭合".into(),
            position: start,
            incomplete: true,
        });
    }

    Ok((word, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_literal_and_expandable_segments() {
        let tokens = lex("echo '$HOME' \"$HOME\"").unwrap();
        let TokenKind::Word(single) = &tokens[1].kind else {
            panic!("expected word")
        };
        let TokenKind::Word(double) = &tokens[2].kind else {
            panic!("expected word")
        };
        assert!(!single.segments[0].expand);
        assert!(double.segments[0].expand);
    }
}
