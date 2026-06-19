use std::path::{Path, PathBuf};

use crate::core::escape;

#[allow(dead_code)]
pub(crate) async fn complete_async(
    shell: &mut crate::core::Shell<impl crate::core::ShellExtensions>,
    line: &str,
    pos: usize,
) -> crate::core::completion::Completions {
    let working_dir = shell.working_dir().to_path_buf();

    // Intentionally ignore any errors that arise.
    let result = shell.complete(line, pos).await;

    let mut completions = result.unwrap_or_else(|_| crate::core::completion::Completions {
        insertion_index: pos,
        delete_count: 0,
        candidates: Vec::new(),
        options: crate::core::completion::ProcessingOptions::default(),
    });

    // Look at the line up to 'pos' to check if we're in an unterminated
    // single or double quote string.
    let mut quote_char: Option<char> = None;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        if i >= pos {
            break;
        }

        if escaped {
            escaped = false;
            continue;
        }

        if let Some(q) = quote_char {
            if c == q {
                quote_char = None;
            }
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' || c == '\"' {
            quote_char = Some(c);
        }
    }

    let completing_end_of_line = pos == line.len();

    // Deduplicate the candidates (retaining order), then postprocess them.
    completions.candidates = completions
        .candidates
        .into_iter()
        .collect::<indexmap::IndexSet<_>>()
        .into_iter()
        .map(|candidate| {
            postprocess_completion_candidate(
                candidate,
                &completions.options,
                working_dir.as_ref(),
                completing_end_of_line,
                quote_char,
            )
        })
        .collect();

    completions
}

#[allow(dead_code)]
fn postprocess_completion_candidate(
    mut candidate: String,
    options: &crate::core::completion::ProcessingOptions,
    working_dir: &Path,
    completing_end_of_line: bool,
    quote_char: Option<char>,
) -> String {
    if options.treat_as_filenames {
        // Check if it's a directory.
        if !crate::core::sys::fs::ends_with_path_separator(&candidate) {
            let candidate_path = Path::new(&candidate);
            let abs_candidate_path = if candidate_path.is_absolute() {
                PathBuf::from(candidate_path)
            } else {
                working_dir.join(candidate_path)
            };

            if abs_candidate_path.is_dir() {
                // Use forward slash: backslash is the shell escape character.
                candidate.push('/');
            }
        }

        if !options.no_autoquote_filenames {
            let quote_mode = match quote_char {
                Some('\'') => escape::QuoteMode::SingleQuote,
                Some('\"') => escape::QuoteMode::DoubleQuote,
                _ => escape::QuoteMode::BackslashEscape,
            };

            candidate = escape::quote_if_needed(&candidate, quote_mode).to_string();
        }
    }
    if completing_end_of_line
        && !options.no_trailing_space_at_end_of_line
        && (!options.treat_as_filenames
            || !crate::core::sys::fs::ends_with_path_separator(&candidate))
    {
        candidate.push(' ');
    }

    candidate
}
