//! Generic syntax highlighting for shell commands.
//!
//! This module provides semantic tagging of shell command strings without
//! imposing any specific styling. Consumers can map the semantic categories
//! to their own color schemes or styles.

use std::str::Chars;

use crate::engine::commands::{self, ResolveOptions, ResolvedCommand};

/// Semantic category for a highlighted span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// Default text
    Default,
    /// Comment text
    Comment,
    /// Arithmetic expression
    Arithmetic,
    /// Parameter expansion (variables, etc.)
    Parameter,
    /// Command substitution
    CommandSubstitution,
    /// Quoted text
    Quoted,
    /// Operator (|, &&, etc.)
    Operator,
    /// Variable assignment
    Assignment,
    /// Hyphen-prefixed option
    HyphenOption,
    /// Function definition
    Function,
    /// Shell keyword
    Keyword,
    /// Builtin command
    Builtin,
    /// Alias
    Alias,
    /// External command (found in PATH)
    ExternalCommand,
    /// Command not found
    NotFoundCommand,
    /// Unknown command (cursor still in token)
    UnknownCommand,
}

/// A highlighted span of text with semantic meaning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    /// Start byte offset in the input string
    pub start: usize,
    /// End byte offset in the input string
    pub end: usize,
    /// Semantic category of this span
    pub kind: HighlightKind,
}

impl HighlightSpan {
    /// Creates a new highlight span.
    #[must_use]
    pub const fn new(start: usize, end: usize, kind: HighlightKind) -> Self {
        Self { start, end, kind }
    }

    /// Returns the text of this span from the input string.
    #[must_use]
    #[allow(clippy::string_slice)]
    pub fn text<'a>(&self, input: &'a str) -> &'a str {
        &input[self.start..self.end]
    }
}

/// Highlights a shell command string, returning semantic spans.
///
/// # Arguments
/// * `shell` - Reference to the shell for context (aliases, functions, builtins, etc.)
/// * `line` - The command string to highlight
/// * `cursor` - Current cursor position (byte offset)
///
/// # Returns
/// A vector of highlighted spans covering the entire input string.
#[must_use]
pub fn highlight_command(
    shell: &crate::engine::Shell,
    line: &str,
    cursor: usize,
) -> Vec<HighlightSpan> {
    let mut highlighter = Highlighter::new(shell, line, cursor);
    highlighter.highlight_program(line, 0);
    highlighter.spans
}

struct Highlighter<'a> {
    shell: &'a crate::engine::Shell,
    cursor: usize,
    spans: Vec<HighlightSpan>,
    remaining_chars: Chars<'a>,
    current_char_index: usize,
    next_missing_kind: Option<HighlightKind>,
}

impl<'a> Highlighter<'a> {
    fn new(shell: &'a crate::engine::Shell, input_line: &'a str, cursor: usize) -> Self {
        Self {
            shell,
            cursor,
            spans: Vec::new(),
            remaining_chars: input_line.chars(),
            current_char_index: 0,
            next_missing_kind: None,
        }
    }

    fn highlight_program(&mut self, line: &str, global_offset: usize) {
        if let Ok(tokens) = crate::parser::tokenize_str_with_options(
            line,
            &(self.shell.parser_options().tokenizer_options()),
        ) {
            let mut saw_command_token = false;
            for token in tokens {
                match token {
                    crate::parser::Token::Operator(_op, token_location) => {
                        self.append_span(
                            HighlightKind::Operator,
                            global_offset + token_location.start.index,
                            global_offset + token_location.end.index,
                        );
                    }
                    crate::parser::Token::Word(w, token_location) => {
                        if let Ok(word_pieces) =
                            crate::parser::word::parse(w.as_str(), &self.shell.parser_options())
                        {
                            let default_text_kind = self.get_kind_for_word(
                                w.as_str(),
                                &token_location,
                                &mut saw_command_token,
                            );

                            for word_piece in word_pieces {
                                self.highlight_word_piece(
                                    word_piece,
                                    default_text_kind,
                                    global_offset + token_location.start.index,
                                );
                            }
                        }
                    }
                }
            }

            self.skip_ahead(global_offset + line.len());
        } else {
            self.append_span(
                HighlightKind::Default,
                global_offset,
                global_offset + line.len(),
            );
        }
    }

    fn highlight_word_piece(
        &mut self,
        word_piece: crate::parser::word::WordPieceWithSource,
        default_text_kind: HighlightKind,
        global_offset: usize,
    ) {
        self.skip_ahead(global_offset + word_piece.start_index);

        match word_piece.piece {
            crate::parser::word::WordPiece::SingleQuotedText(_)
            | crate::parser::word::WordPiece::AnsiCQuotedText(_)
            | crate::parser::word::WordPiece::EscapeSequence(_) => {
                self.append_span(
                    HighlightKind::Quoted,
                    global_offset + word_piece.start_index,
                    global_offset + word_piece.end_index,
                );
            }
            crate::parser::word::WordPiece::DoubleQuotedSequence(subpieces)
            | crate::parser::word::WordPiece::GettextDoubleQuotedSequence(subpieces) => {
                self.set_next_missing_kind(HighlightKind::Quoted);
                for subpiece in subpieces {
                    self.highlight_word_piece(subpiece, HighlightKind::Quoted, global_offset);
                }
                self.set_next_missing_kind(HighlightKind::Quoted);
            }
            crate::parser::word::WordPiece::ParameterExpansion(_)
            | crate::parser::word::WordPiece::TildeExpansion(_) => {
                self.append_span(
                    HighlightKind::Parameter,
                    global_offset + word_piece.start_index,
                    global_offset + word_piece.end_index,
                );
            }
            crate::parser::word::WordPiece::BackquotedCommandSubstitution(command) => {
                self.set_next_missing_kind(HighlightKind::CommandSubstitution);
                self.highlight_program(
                    command.as_str(),
                    global_offset + word_piece.start_index + 1, /* account for opening backtick */
                );
                self.set_next_missing_kind(HighlightKind::CommandSubstitution);
            }
            crate::parser::word::WordPiece::CommandSubstitution(command) => {
                self.set_next_missing_kind(HighlightKind::CommandSubstitution);
                self.highlight_program(
                    command.as_str(),
                    global_offset + word_piece.start_index + 2, /* account for opening $( */
                );
                self.set_next_missing_kind(HighlightKind::CommandSubstitution);
            }
            crate::parser::word::WordPiece::ArithmeticExpression(_) => {
                // TODO(highlighting): Consider individually highlighting pieces of the expression
                // itself.
                self.append_span(
                    HighlightKind::Arithmetic,
                    global_offset + word_piece.start_index,
                    global_offset + word_piece.end_index,
                );
            }
            crate::parser::word::WordPiece::Text(_text) => {
                self.append_span(
                    default_text_kind,
                    global_offset + word_piece.start_index,
                    global_offset + word_piece.end_index,
                );
            }
        }

        self.skip_ahead(global_offset + word_piece.end_index);
    }

    fn append_span(&mut self, kind: HighlightKind, start: usize, end: usize) {
        // See if we need to cover a gap between this substring and the one that preceded it.
        if start > self.current_char_index {
            let missing_kind = self.next_missing_kind.unwrap_or(HighlightKind::Comment);
            let gap_len = start - self.current_char_index;

            // Skip characters in the gap
            for _ in 0..gap_len {
                self.remaining_chars.next();
            }

            self.spans.push(HighlightSpan::new(
                self.current_char_index,
                start,
                missing_kind,
            ));
            self.current_char_index = start;
        }

        if end > start {
            // Skip characters in this span
            for _ in 0..(end - start) {
                self.remaining_chars.next();
            }

            self.spans.push(HighlightSpan::new(start, end, kind));
        }

        self.current_char_index = end;
    }

    fn skip_ahead(&mut self, dest: usize) {
        // Append a no-op span to make sure we cover any trailing gaps in the input line not
        // otherwise styled.
        self.append_span(HighlightKind::Default, dest, dest);
    }

    const fn set_next_missing_kind(&mut self, kind: HighlightKind) {
        self.next_missing_kind = Some(kind);
    }

    fn get_kind_for_word(
        &self,
        w: &str,
        token_location: &crate::parser::SourceSpan,
        saw_command_token: &mut bool,
    ) -> HighlightKind {
        if !*saw_command_token {
            if w.contains('=') {
                HighlightKind::Assignment
            } else {
                *saw_command_token = true;
                self.classify_possible_command(w, token_location)
            }
        } else {
            if self.shell.is_keyword(w) {
                HighlightKind::Keyword
            } else if w.starts_with('-') {
                HighlightKind::HyphenOption
            } else {
                HighlightKind::Default
            }
        }
    }

    fn classify_possible_command(
        &self,
        name: &str,
        token_location: &crate::parser::SourceSpan,
    ) -> HighlightKind {
        let non_path_options = ResolveOptions {
            include_aliases: true,
            include_keywords: true,
            include_functions: true,
            include_builtins: true,
            include_disabled_builtins: true,
            include_path: false,
            include_hashed: false,
            all_locations: false,
            force_path_search: false,
            use_default_path: false,
            literal_path_with_separator: false,
        };

        if let Some(resolved) = commands::resolve_command(self.shell, name, &non_path_options)
            .into_iter()
            .next()
        {
            return Self::highlight_kind_for_resolved_command(resolved);
        }

        // Short-circuit if the cursor is still in this token.
        if (self.cursor >= token_location.start.index) && (self.cursor <= token_location.end.index)
        {
            return HighlightKind::UnknownCommand;
        }

        let path_options = ResolveOptions {
            include_aliases: false,
            include_keywords: false,
            include_functions: false,
            include_builtins: false,
            include_disabled_builtins: false,
            include_path: true,
            include_hashed: false,
            all_locations: false,
            force_path_search: true,
            use_default_path: false,
            literal_path_with_separator: false,
        };

        commands::resolve_command(self.shell, name, &path_options)
            .into_iter()
            .next()
            .map(Self::highlight_kind_for_resolved_command)
            .unwrap_or(HighlightKind::NotFoundCommand)
    }

    fn highlight_kind_for_resolved_command(resolved: ResolvedCommand<'_>) -> HighlightKind {
        match resolved {
            ResolvedCommand::Alias(_) => HighlightKind::Alias,
            ResolvedCommand::Keyword => HighlightKind::Keyword,
            ResolvedCommand::Function(_) => HighlightKind::Function,
            ResolvedCommand::Builtin => HighlightKind::Builtin,
            ResolvedCommand::External { .. } => HighlightKind::ExternalCommand,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[compio::test]
    async fn test_highlight_simple_command() {
        let shell = crate::engine::Shell::builder().build().await.unwrap();
        let line = "somecommand hello";
        // Use cursor position at the end so we get final highlighting
        let spans = highlight_command(&shell, line, line.len());

        // Should have at least 2 spans
        assert!(!spans.is_empty());

        // Verify highlighting produces spans that cover the input
        let total_covered: usize = spans.iter().map(|s| s.end - s.start).sum();
        assert_eq!(total_covered, line.len(), "Spans should cover entire input");

        // The command should be classified as something (NotFound, External, etc.)
        let cmd_span = spans.iter().find(|s| s.text(line) == "somecommand");
        assert!(cmd_span.is_some(), "Should have a span for the command");
    }

    #[compio::test]
    async fn test_highlight_quoted_string() {
        let shell = crate::engine::Shell::builder().build().await.unwrap();
        let line = r#"echo "hello world""#;
        let spans = highlight_command(&shell, line, 0);

        // Should have spans for: echo, space, "hello world"
        assert!(!spans.is_empty());

        // Check that quoted parts are marked as Quoted
        assert!(spans.iter().any(|s| s.kind == HighlightKind::Quoted));
    }

    #[compio::test]
    async fn test_highlight_parameter_expansion() {
        let shell = crate::engine::Shell::builder().build().await.unwrap();
        let line = "echo $HOME";
        let spans = highlight_command(&shell, line, 0);

        // Should have spans including a parameter expansion
        assert!(spans.iter().any(|s| s.kind == HighlightKind::Parameter));
    }

    #[compio::test]
    async fn test_highlight_covers_entire_input() {
        let shell = crate::engine::Shell::builder().build().await.unwrap();
        let line = "echo hello world";
        let spans = highlight_command(&shell, line, 0);

        // Verify that spans cover the entire input (no gaps)
        let mut covered = vec![false; line.len()];
        for span in &spans {
            for item in covered.iter_mut().take(span.end).skip(span.start) {
                *item = true;
            }
        }

        assert!(covered.iter().all(|&c| c), "Not all characters are covered");
    }
}
