#![allow(clippy::needless_pass_by_value)]

use std::borrow::Cow;
use std::cell::RefCell;

use crate::engine::error;

type RegexCacheKey = (String, bool, bool);

const MAX_PATTERN_BYTES: usize = 256 * 1024;
const MAX_GENERATED_REGEX_BYTES: usize = 1024 * 1024;
const MAX_REGEX_NESTING_DEPTH: usize = 64;

thread_local! {
    static REGEX_CACHE: RefCell<crate::engine::cache::FixedCache<RegexCacheKey, fancy_regex::Regex>> =
        RefCell::new(crate::engine::cache::FixedCache::new(64));
}

/// Represents a piece of a regular expression.
#[derive(Clone, Debug)]
pub(crate) enum RegexPiece {
    /// A pattern that should be interpreted as a regular expression.
    Pattern(String),
    /// A literal string that should be matched exactly.
    Literal(String),
}

impl RegexPiece {
    fn to_regex_str(&self) -> Cow<'_, str> {
        match self {
            Self::Pattern(s) => Cow::Borrowed(s.as_str()),
            Self::Literal(s) => escape_literal_regex_piece(s.as_str()),
        }
    }
}

type RegexWord = Vec<RegexPiece>;

/// Encapsulates a regular expression usable in the shell.
#[derive(Clone, Debug)]
pub struct Regex {
    pieces: RegexWord,
    case_insensitive: bool,
    multiline: bool,
}

impl From<RegexWord> for Regex {
    fn from(pieces: RegexWord) -> Self {
        Self {
            pieces,
            case_insensitive: false,
            multiline: false,
        }
    }
}

impl Regex {
    /// Sets the regular expression's case sensitivity.
    ///
    /// # Arguments
    ///
    /// * `value` - The new case sensitivity value.
    pub const fn set_case_insensitive(mut self, value: bool) -> Self {
        self.case_insensitive = value;
        self
    }

    /// Enables (or disables) multiline support for this pattern.
    /// This enables matching across lines as well as enables `.`
    /// to match newline characters.
    ///
    /// # Arguments
    ///
    /// * `value` - The new multiline value.
    pub const fn set_multiline(mut self, value: bool) -> Self {
        self.multiline = value;
        self
    }

    /// Computes if the regular expression matches the given string.
    ///
    /// # Arguments
    ///
    /// * `value` - The string to check for a match.
    pub fn matches(&self, value: &str) -> Result<Option<Vec<Option<String>>>, error::Error> {
        let pattern_bytes = self.pieces.iter().fold(0usize, |total, piece| {
            let piece_bytes = match piece {
                RegexPiece::Pattern(value) | RegexPiece::Literal(value) => value.len(),
            };
            total.saturating_add(piece_bytes)
        });
        check_regex_size(pattern_bytes, MAX_PATTERN_BYTES, "pattern", "")?;

        let regex_pattern: String = self
            .pieces
            .iter()
            .map(|piece| piece.to_regex_str())
            .collect();

        let re = compile_regex(regex_pattern, self.case_insensitive, self.multiline)?;

        Ok(re.captures(value)?.map(|captures| {
            captures
                .iter()
                .map(|c| c.map(|m| m.as_str().to_owned()))
                .collect()
        }))
    }
}

pub(crate) fn compile_regex(
    regex_str: String,
    case_insensitive: bool,
    multiline: bool,
) -> Result<fancy_regex::Regex, error::Error> {
    check_generated_regex(&regex_str)?;
    let key_bytes = regex_str.len();

    REGEX_CACHE.with(|cache| {
        crate::engine::cache::get_or_try_insert_with(
            cache,
            (regex_str, case_insensitive, multiline),
            key_bytes,
            |(regex_str, case_insensitive, multiline)| {
                // Shell 支持的部分 regex 不能直接由 `fancy_regex` 处理, 此处补充缺失的转义.
                let mut regex_str = add_missing_escape_chars_to_regex(regex_str.as_str());

                // `fancy_regex` 未通过 RegexBuilder 暴露所需的 multiline 组合, 因此添加 flag 前缀.
                if *multiline {
                    let updated_str = std::format!("(?ms){regex_str}");
                    regex_str = updated_str.into();
                }

                check_generated_regex(regex_str.as_ref())?;

                let mut builder = fancy_regex::RegexBuilder::new(regex_str.as_ref());
                builder.case_insensitive(*case_insensitive);

                builder.build().map_err(|e| {
                    error::Error::from(error::ErrorKind::InvalidRegexError(
                        e,
                        regex_str.to_string(),
                    ))
                })
            },
        )
    })
}

fn check_regex_size(
    actual_bytes: usize,
    max_bytes: usize,
    kind: &str,
    expression: &str,
) -> Result<(), error::Error> {
    if actual_bytes <= max_bytes {
        return Ok(());
    }

    Err(regex_limit_error(
        max_bytes,
        fancy_regex::ParseError::GeneralParseError(std::format!(
            "{kind} size {actual_bytes} bytes exceeds the {max_bytes} byte limit"
        )),
        expression,
    ))
}

fn check_generated_regex(regex: &str) -> Result<(), error::Error> {
    check_regex_size(
        regex.len(),
        MAX_GENERATED_REGEX_BYTES,
        "generated regex",
        regex,
    )?;

    let mut depth = 0usize;
    let mut escaped = false;
    let mut in_character_class = false;

    for (offset, c) in regex.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        match c {
            '\\' => escaped = true,
            '[' if !in_character_class => in_character_class = true,
            ']' if in_character_class => in_character_class = false,
            '(' if !in_character_class => {
                depth += 1;
                if depth >= MAX_REGEX_NESTING_DEPTH {
                    return Err(regex_limit_error(
                        offset,
                        fancy_regex::ParseError::RecursionExceeded,
                        regex,
                    ));
                }
            }
            ')' if !in_character_class => depth = depth.saturating_sub(1),
            _ => {}
        }
    }

    Ok(())
}

fn regex_limit_error(
    position: usize,
    parse_error: fancy_regex::ParseError,
    expression: &str,
) -> error::Error {
    let expression = if expression.len() <= 256 {
        expression.to_owned()
    } else {
        std::format!("<{} byte expression omitted>", expression.len())
    };
    error::Error::from(error::ErrorKind::InvalidRegexError(
        fancy_regex::Error::ParseError(position, parse_error),
        expression,
    ))
}

fn add_missing_escape_chars_to_regex(s: &str) -> Cow<'_, str> {
    // We may see a character class with an unescaped '[' (open bracket) character. We need
    // to escape that character.
    let mut in_escape = false;
    let mut in_brackets = false;
    let mut insertion_positions = vec![];

    let mut peekable = s.char_indices().peekable();
    while let Some((byte_offset, c)) = peekable.next() {
        let next_is_colon = peekable.peek().is_some_and(|(_, c)| *c == ':');

        match c {
            '[' if !in_escape && !in_brackets => {
                in_brackets = true;
            }
            '[' if !in_escape && in_brackets && !next_is_colon => {
                // Need to escape.
                insertion_positions.push(byte_offset);
            }
            ']' if !in_escape && in_brackets => {
                in_brackets = false;
            }
            _ => (),
        }

        in_escape = !in_escape && c == '\\';
    }

    if insertion_positions.is_empty() {
        return s.into();
    }

    let mut updated = s.to_owned();
    for pos in insertion_positions.iter().rev() {
        updated.insert(*pos, '\\');
    }

    updated.into()
}

fn escape_literal_regex_piece(s: &str) -> Cow<'_, str> {
    let mut result = String::new();

    for c in s.chars() {
        match c {
            c if regex_char_is_special(c) => {
                result.push('\\');
                result.push(c);
            }
            c => result.push(c),
        }
    }

    result.into()
}

pub(crate) const fn regex_char_is_special(c: char) -> bool {
    matches!(
        c,
        '\\' | '^' | '$' | '.' | '|' | '?' | '*' | '+' | '(' | ')' | '[' | ']' | '{' | '}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_missing_escape_chars_to_regex() {
        // Negative cases -- where we don't need to escape.
        assert_eq!(add_missing_escape_chars_to_regex("a[b]"), "a[b]");
        assert_eq!(add_missing_escape_chars_to_regex(r"a\[b\]"), r"a\[b\]");
        assert_eq!(add_missing_escape_chars_to_regex(r"a[b\[]"), r"a[b\[]");

        // Positive case -- where we need to escape.
        assert_eq!(add_missing_escape_chars_to_regex(r"a[b[]"), r"a[b\[]");
        assert_eq!(add_missing_escape_chars_to_regex(r"a[[]"), r"a[\[]");
    }

    #[test]
    fn normal_regexes_are_cached() {
        REGEX_CACHE.with(|cache| cache.borrow_mut().clear());

        compile_regex("a+".to_owned(), false, false).unwrap();
        compile_regex("a+".to_owned(), false, false).unwrap();

        REGEX_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 1));
    }

    #[test]
    fn large_regexes_are_accepted_but_not_cached() {
        REGEX_CACHE.with(|cache| cache.borrow_mut().clear());
        let regex = "a".repeat(crate::engine::cache::MAX_CACHE_KEY_BYTES + 1);

        compile_regex(regex, false, false).unwrap();

        REGEX_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 0));
    }

    #[test]
    fn maliciously_large_patterns_return_a_structured_error() {
        let regex = Regex::from(vec![RegexPiece::Pattern("a".repeat(MAX_PATTERN_BYTES + 1))]);

        let error = regex.matches("").unwrap_err();

        assert!(error.to_string().contains("pattern size"));
    }

    #[test]
    fn maliciously_large_generated_regexes_return_a_structured_error() {
        let error =
            compile_regex("a".repeat(MAX_GENERATED_REGEX_BYTES + 1), false, false).unwrap_err();

        assert!(error.to_string().contains("generated regex size"));
    }

    #[test]
    fn deeply_nested_regexes_return_a_structured_error() {
        let regex = std::format!(
            "{}a{}",
            "(".repeat(MAX_REGEX_NESTING_DEPTH),
            ")".repeat(MAX_REGEX_NESTING_DEPTH)
        );

        let error = compile_regex(regex, false, false).unwrap_err();

        assert!(error.to_string().contains("Pattern too deeply nested"));
    }
}
