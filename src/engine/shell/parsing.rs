//! Parsing for shell instances.

use std::{cell::RefCell, io::Read};

type ParseStringCacheKey = (String, crate::parser::ParserOptions);

thread_local! {
    static PARSE_STRING_CACHE: RefCell<crate::engine::cache::FixedCache<ParseStringCacheKey, crate::parser::ast::Program>> =
        RefCell::new(crate::engine::cache::FixedCache::new(64));
}

use crate::engine::{Shell, trace_categories};

impl Shell {
    /// Parses the given reader as a shell program, returning the resulting Abstract Syntax Tree
    /// for the program.
    pub fn parse<R: Read>(
        &self,
        reader: R,
    ) -> Result<crate::parser::ast::Program, crate::parser::ParseError> {
        let mut parser = create_parser(reader, &self.parser_options());

        log::debug!(target: trace_categories::PARSE, "Parsing reader as program...");
        parser.parse_program()
    }

    /// Parses the given string as a shell program, returning the resulting Abstract Syntax Tree
    /// for the program.
    ///
    /// # Arguments
    ///
    /// * `s` - The string to parse as a program.
    pub fn parse_string<S: Into<String>>(
        &self,
        s: S,
    ) -> Result<crate::parser::ast::Program, crate::parser::ParseError> {
        parse_string_impl(s.into(), self.parser_options())
    }

    /// Returns the options that should be used for parsing shell programs; reflects
    /// the current configuration state of the shell and may change over time.
    pub const fn parser_options(&self) -> crate::parser::ParserOptions {
        crate::parser::ParserOptions {
            enable_extended_globbing: self.options.extended_globbing,
            tilde_expansion_at_word_start: true,
            tilde_expansion_after_colon: false,
        }
    }
}

fn parse_string_impl(
    s: String,
    parser_options: crate::parser::ParserOptions,
) -> Result<crate::parser::ast::Program, crate::parser::ParseError> {
    let key_bytes = s.len();
    PARSE_STRING_CACHE.with(|cache| {
        crate::engine::cache::get_or_try_insert_with(cache, (s, parser_options), key_bytes, |key| {
            let (s, parser_options) = key;
            let mut parser = create_parser(s.as_bytes(), parser_options);

            log::debug!(target: trace_categories::PARSE, "Parsing string as program...");
            parser.parse_program()
        })
    })
}

pub(super) fn create_parser<R: Read>(
    r: R,
    parser_options: &crate::parser::ParserOptions,
) -> crate::parser::Parser<std::io::BufReader<R>> {
    let reader = std::io::BufReader::new(r);
    crate::parser::Parser::new(reader, parser_options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_programs_are_cached() {
        PARSE_STRING_CACHE.with(|cache| cache.borrow_mut().clear());

        parse_string_impl(
            "echo cached".to_owned(),
            crate::parser::ParserOptions::default(),
        )
        .unwrap();

        PARSE_STRING_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 1));
    }

    #[test]
    fn oversized_programs_are_not_cached() {
        PARSE_STRING_CACHE.with(|cache| cache.borrow_mut().clear());
        let program = std::format!("#{}", "x".repeat(crate::engine::cache::MAX_CACHE_KEY_BYTES));

        parse_string_impl(program, crate::parser::ParserOptions::default()).unwrap();

        PARSE_STRING_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 0));
    }
}
