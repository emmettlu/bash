use crate::engine::{
    ExecutionParameters, error, expansion,
    shell::Shell,
    sys::{self, users},
};
use std::{cell::RefCell, path::Path};

thread_local! {
    static PROMPT_PARSE_CACHE: RefCell<crate::engine::cache::FixedCache<String, Vec<crate::parser::prompt::PromptPiece>>> =
        RefCell::new(crate::engine::cache::FixedCache::new(64));
}

const VERSION_MAJOR: &str = env!("CARGO_PKG_VERSION_MAJOR");
const VERSION_MINOR: &str = env!("CARGO_PKG_VERSION_MINOR");
const VERSION_PATCH: &str = env!("CARGO_PKG_VERSION_PATCH");

pub(crate) async fn expand_prompt(
    shell: &mut Shell,
    params: &ExecutionParameters,
    spec: String,
) -> Result<String, error::Error> {
    // Parse the prompt spec into its pieces.
    let prompt_pieces = parse_prompt(spec)?;

    // Now, render each piece.
    let mut formatted_prompt = String::new();
    for piece in prompt_pieces {
        let needs_escaping = matches!(
            piece,
            crate::parser::prompt::PromptPiece::EscapedSequence(_)
                | crate::parser::prompt::PromptPiece::DollarOrPound
        );

        let formatted_piece = format_prompt_piece(shell, piece)?;

        if shell.options().expand_prompt_strings && needs_escaping {
            formatted_prompt.push('\\');
        }

        formatted_prompt.push_str(&formatted_piece);
    }

    if shell.options().expand_prompt_strings {
        // Now expand any remaining escape sequences, but without tilde-expansion.
        let options = expansion::ExpanderOptions {
            tilde_expand: false,
            ..Default::default()
        };
        formatted_prompt =
            expansion::basic_expand_word_with_options(shell, params, &formatted_prompt, &options)
                .await?;
    }

    Ok(formatted_prompt)
}

fn parse_prompt(
    spec: String,
) -> Result<Vec<crate::parser::prompt::PromptPiece>, crate::parser::WordParseError> {
    let key_bytes = spec.len();
    PROMPT_PARSE_CACHE.with(|cache| {
        crate::engine::cache::get_or_try_insert_with(cache, spec, key_bytes, |spec| {
            crate::parser::prompt::parse(spec.as_str())
        })
    })
}

fn format_prompt_piece(
    shell: &Shell,
    piece: crate::parser::prompt::PromptPiece,
) -> Result<String, error::Error> {
    let formatted = match piece {
        crate::parser::prompt::PromptPiece::EscapedSequence(s) => s,
        crate::parser::prompt::PromptPiece::Literal(l) => l,
        crate::parser::prompt::PromptPiece::AsciiCharacter(c) => {
            char::from_u32(c).map_or_else(String::new, |c| c.to_string())
        }
        crate::parser::prompt::PromptPiece::Backslash => "\\".to_owned(),
        crate::parser::prompt::PromptPiece::BellCharacter => "\x07".to_owned(),
        crate::parser::prompt::PromptPiece::CarriageReturn => "\r".to_owned(),
        crate::parser::prompt::PromptPiece::CurrentCommandNumber => {
            return error::unimp("prompt: current command number");
        }
        crate::parser::prompt::PromptPiece::CurrentHistoryNumber => {
            return error::unimp("prompt: current history number");
        }
        crate::parser::prompt::PromptPiece::CurrentUser => users::get_current_username()?,
        crate::parser::prompt::PromptPiece::CurrentWorkingDirectory {
            tilde_replaced,
            basename,
        } => format_current_working_directory(shell, tilde_replaced, basename),
        crate::parser::prompt::PromptPiece::Date(format) => {
            crate::engine::timefmt::format_date(&nanotime::NanoTime::now(), &format)
        }
        crate::parser::prompt::PromptPiece::DollarOrPound => {
            if users::is_root() {
                "#".to_owned()
            } else {
                "$".to_owned()
            }
        }
        // NOTE: We mimic bash and convert \[ into \001, a.k.a. RL_PROMPT_START_IGNORE.
        // It will need to get removed before it's actually displayed. While present it
        // also has the important (compatible) side effect of ensuring the text on either
        // side of it is not concatenated together, potentially resulting in incompatible
        // variable expansions. Also, we *only* do this if the shell is interactive.
        crate::parser::prompt::PromptPiece::EndNonPrintingSequence => {
            if shell.options().interactive {
                "\x02".to_owned()
            } else {
                String::new()
            }
        }
        crate::parser::prompt::PromptPiece::EscapeCharacter => "\x1b".to_owned(),
        crate::parser::prompt::PromptPiece::Hostname {
            only_up_to_first_dot,
        } => {
            let hn = sys::network::get_hostname()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if only_up_to_first_dot && let Some((first, _)) = hn.split_once('.') {
                return Ok(first.to_owned());
            }
            hn
        }
        crate::parser::prompt::PromptPiece::Newline => "\n".to_owned(),
        crate::parser::prompt::PromptPiece::NumberOfManagedJobs => {
            shell.jobs().jobs.len().to_string()
        }
        crate::parser::prompt::PromptPiece::ShellBaseName => {
            if let Some(shell_name) = shell.current_shell_name() {
                Path::new(shell_name.as_ref())
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
        crate::parser::prompt::PromptPiece::ShellRelease => {
            std::format!("{VERSION_MAJOR}.{VERSION_MINOR}.{VERSION_PATCH}")
        }
        crate::parser::prompt::PromptPiece::ShellVersion => {
            std::format!("{VERSION_MAJOR}.{VERSION_MINOR}")
        }
        // NOTE: See above note for EndNonPrintingSequence
        crate::parser::prompt::PromptPiece::StartNonPrintingSequence => {
            if shell.options().interactive {
                "\x01".to_owned()
            } else {
                String::new()
            }
        }
        crate::parser::prompt::PromptPiece::TerminalDeviceBaseName => {
            sys::terminal::try_get_terminal_device_path()
                .and_then(|p| p.file_name().map(|s| s.to_string_lossy().to_string()))
                .unwrap_or_default()
        }
        crate::parser::prompt::PromptPiece::Time(time_fmt) => {
            crate::engine::timefmt::format_time(&nanotime::NanoTime::now(), &time_fmt)
        }
    };

    Ok(formatted)
}

fn format_current_working_directory(shell: &Shell, tilde_replaced: bool, basename: bool) -> String {
    let mut working_dir_str = shell.working_dir().to_string_lossy().to_string();

    if tilde_replaced {
        working_dir_str = shell.tilde_shorten(working_dir_str);
    }

    if basename && let Some(filename) = Path::new(&working_dir_str).file_name() {
        working_dir_str = filename.to_string_lossy().to_string();
    }

    if cfg!(windows) {
        working_dir_str = working_dir_str.replace('\\', "/");
    }

    working_dir_str
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_prompts_are_cached() {
        PROMPT_PARSE_CACHE.with(|cache| cache.borrow_mut().clear());

        parse_prompt(r"\u@\h:\w\$ ".to_owned()).unwrap();

        PROMPT_PARSE_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 1));
    }

    #[test]
    fn oversized_prompts_are_not_cached() {
        PROMPT_PARSE_CACHE.with(|cache| cache.borrow_mut().clear());
        let prompt = "x".repeat(crate::engine::cache::MAX_CACHE_KEY_BYTES + 1);

        parse_prompt(prompt).unwrap();

        PROMPT_PARSE_CACHE.with(|cache| assert_eq!(cache.borrow().len(), 0));
    }
}
