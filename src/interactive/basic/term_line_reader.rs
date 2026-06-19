//
// This module is intentionally limited, and does not have all the bells and whistles. We want
// enough here that we can use it in the basic shell for (p)expect/pty-style testing of
// completion, and without using VT100-style escape sequences for cursor movement and display.
//

use std::io::Write;

use crate::interactive::win_term::{self, KeyCode};
use crate::interactive::{ReadResult, ShellError};

const BACKSPACE: char = 8u8 as char;

pub(crate) struct TermLineReader {
    _console_mode: ConsoleModeGuard,
}

struct ConsoleModeGuard {
    original_mode: u32,
}

impl ConsoleModeGuard {
    fn new() -> Result<Self, ShellError> {
        let original_mode = win_term::enable_raw_mode()?;
        Ok(Self { original_mode })
    }
}

impl Drop for ConsoleModeGuard {
    fn drop(&mut self) {
        let _ = win_term::restore_console_mode(self.original_mode);
    }
}

impl TermLineReader {
    pub fn new() -> Result<Self, ShellError> {
        Ok(Self {
            _console_mode: ConsoleModeGuard::new()?,
        })
    }
}

impl super::LineReader for TermLineReader {
    fn read_line(
        &self,
        prompt: Option<&str>,
        mut completion_handler: impl FnMut(
            &str,
            usize,
        )
            -> Result<crate::core::completion::Completions, ShellError>,
    ) -> Result<ReadResult, ShellError> {
        let mut state = ReadLineState::new(prompt);
        state.display_prompt()?;

        loop {
            let key_event = win_term::read_key_event()?;
            if let Some(result) = state.on_key(key_event, &mut completion_handler)? {
                return Ok(result);
            }
        }
    }
}

struct ReadLineState<'a> {
    // Current line of input
    line: String,
    // Current position of cursor, expressed as a byte offset from the
    // start of `line`. We maintain the invariant that it will always
    // be at a clean character boundary.
    cursor: usize,
    // Current prompt to use.
    prompt: Option<&'a str>,
    completion_menu: Option<CompletionMenu>,
}

struct CompletionMenu {
    completions: crate::core::completion::Completions,
    selected: usize,
    rendered_lines: usize,
    cursor_visibility: Option<win_term::CursorVisibilityGuard>,
}

impl<'a> ReadLineState<'a> {
    const fn new(prompt: Option<&'a str>) -> Self {
        Self {
            line: String::new(),
            cursor: 0,
            prompt,
            completion_menu: None,
        }
    }

    pub fn display_prompt(&self) -> Result<(), ShellError> {
        if let Some(prompt) = self.prompt {
            eprint!("{prompt}");
            std::io::stderr().flush()?;
        }

        Ok(())
    }

    fn on_key(
        &mut self,
        event: win_term::KeyEvent,
        mut completion_handler: impl FnMut(
            &str,
            usize,
        )
            -> Result<crate::core::completion::Completions, ShellError>,
    ) -> Result<Option<ReadResult>, ShellError> {
        match (&event.modifiers, &event.code) {
            (_, KeyCode::Enter) if self.completion_menu.is_some() => {
                self.accept_completion_selection()?;
            }
            (_, KeyCode::Enter) => {
                self.clear_completion_menu()?;
                Self::display_newline()?;
                self.line.push('\n');
                let line = std::mem::take(&mut self.line);
                return Ok(Some(ReadResult::Input(line)));
            }
            (mods, KeyCode::Char('j')) if mods.ctrl => {
                self.clear_completion_menu()?;
                Self::display_newline()?;
                self.line.push('\n');
                let line = std::mem::take(&mut self.line);
                return Ok(Some(ReadResult::Input(line)));
            }
            (mods, KeyCode::Char(c)) if !mods.ctrl && !mods.alt => {
                self.clear_completion_menu()?;
                self.on_char(*c)?;
            }
            (mods, KeyCode::Char('c')) if mods.ctrl => {
                self.clear_completion_menu()?;
                eprintln!("^C");
                return Ok(Some(ReadResult::Interrupted));
            }
            (mods, KeyCode::Char('d')) if mods.ctrl && self.line.is_empty() => {
                self.clear_completion_menu()?;
                Self::display_newline()?;
                return Ok(Some(ReadResult::Eof));
            }
            (mods, KeyCode::Char('l')) if mods.ctrl => {
                self.clear_completion_menu()?;
                self.clear_screen()?;
            }
            (_, KeyCode::Backspace) => {
                self.clear_completion_menu()?;
                self.backspace()?;
            }
            (_, KeyCode::Left) => {
                self.clear_completion_menu()?;
                self.move_cursor_left()?;
            }
            (_, KeyCode::Right) => {
                self.clear_completion_menu()?;
                self.move_cursor_right()?;
            }
            (_, KeyCode::Tab) => {
                self.handle_tab(&mut completion_handler)?;
            }
            _ => (),
        }

        Ok(None)
    }

    fn on_char(&mut self, c: char) -> Result<(), ShellError> {
        let insertion_index = self.cursor;
        self.line.insert(self.cursor, c);
        self.cursor += c.len_utf8();

        eprint!("{}", &self.line[insertion_index..]);
        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, self.line.len() - self.cursor)
        );
        std::io::stderr().flush()?;

        Ok(())
    }

    fn display_newline() -> Result<(), ShellError> {
        eprintln!();
        std::io::stderr().flush()?;

        Ok(())
    }

    fn clear_screen(&self) -> Result<(), ShellError> {
        win_term::clear_screen()?;
        self.display_prompt()?;
        eprint!("{}", self.line.as_str());
        std::io::stderr().flush()?;
        Ok(())
    }

    #[allow(clippy::string_slice, reason = "it's calculated based on char indices")]
    fn backspace(&mut self) -> Result<(), ShellError> {
        if self.cursor == 0 {
            return Ok(());
        }

        let deleted_char_start = self.prev_char_boundary(self.cursor);
        self.line.drain(deleted_char_start..self.cursor);
        self.cursor = deleted_char_start;

        eprint!("{BACKSPACE}");
        eprint!("{} ", &self.line[self.cursor..]);
        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, self.line.len() + 1 - self.cursor)
        );

        std::io::stderr().flush()?;
        Ok(())
    }

    fn move_cursor_left(&mut self) -> Result<(), ShellError> {
        if self.cursor == 0 {
            return Ok(());
        }

        eprint!("{BACKSPACE}");
        std::io::stderr().flush()?;
        self.cursor = self.prev_char_boundary(self.cursor);

        Ok(())
    }

    fn move_cursor_right(&mut self) -> Result<(), ShellError> {
        if self.cursor >= self.line.len() {
            return Ok(());
        }

        let next = self.next_char_boundary(self.cursor);
        eprint!("{}", &self.line[self.cursor..next]);
        std::io::stderr().flush()?;
        self.cursor = next;

        Ok(())
    }

    fn prev_char_boundary(&self, index: usize) -> usize {
        self.line[..index]
            .char_indices()
            .last()
            .map_or(0, |(idx, _)| idx)
    }

    fn next_char_boundary(&self, index: usize) -> usize {
        let mut iter = self.line[index..].char_indices();
        let _ = iter.next();
        iter.next()
            .map_or(self.line.len(), |(offset, _)| index + offset)
    }

    fn handle_tab(
        &mut self,
        completion_handler: &mut impl FnMut(
            &str,
            usize,
        )
            -> Result<crate::core::completion::Completions, ShellError>,
    ) -> Result<(), ShellError> {
        if self.completion_menu.is_some() {
            if let Some(menu) = self.completion_menu.as_mut() {
                menu.selected = (menu.selected + 1) % menu.completions.candidates.len();
            }
            return self.render_completion_menu();
        }

        let completions = completion_handler(self.line.as_str(), self.cursor)?;
        self.handle_completions(completions)
    }

    fn handle_completions(
        &mut self,
        completions: crate::core::completion::Completions,
    ) -> Result<(), ShellError> {
        if completions.candidates.is_empty() {
            Ok(())
        } else if completions.candidates.len() == 1 {
            self.handle_single_completion(&completions)
        } else {
            self.completion_menu = Some(CompletionMenu {
                completions,
                selected: 0,
                rendered_lines: 0,
                cursor_visibility: None,
            });
            self.render_completion_menu()
        }
    }

    #[expect(
        clippy::string_slice,
        reason = "all offsets are expected to be at char boundaries"
    )]
    fn handle_single_completion(
        &mut self,
        completions: &crate::core::completion::Completions,
    ) -> Result<(), ShellError> {
        let Some(candidate) = completions.candidates.first() else {
            return Ok(());
        };

        if completions.insertion_index + completions.delete_count != self.cursor {
            return Ok(());
        }

        let mut delete_count = completions.delete_count;
        let mut redisplay_offset = completions.insertion_index;

        if delete_count > 0
            && candidate.starts_with(&self.line[redisplay_offset..redisplay_offset + delete_count])
        {
            redisplay_offset += delete_count;
            delete_count = 0;
        }

        let mut updated_line = self.line.clone();
        updated_line.truncate(completions.insertion_index);
        updated_line.push_str(candidate);
        updated_line.push_str(&self.line[self.cursor..]);
        self.line = updated_line;

        self.cursor = completions.insertion_index + candidate.len();

        let move_left = repeated_char_str(BACKSPACE, delete_count);
        eprint!("{move_left}{}", &self.line[redisplay_offset..]);

        eprint!(
            "{}",
            repeated_char_str(BACKSPACE, self.line.len() - self.cursor)
        );

        std::io::stderr().flush()?;

        Ok(())
    }

    fn render_completion_menu(&mut self) -> Result<(), ShellError> {
        const MAX_COLUMNS: usize = 4;

        let Some(menu) = self.completion_menu.as_mut() else {
            return Ok(());
        };

        if menu.cursor_visibility.is_none() {
            menu.cursor_visibility = win_term::hide_cursor().ok();
        }

        let input_cursor = win_term::get_cursor_position()?;
        let completions_start_y = input_cursor.y.saturating_add(1);
        let (buffer_width, buffer_height) = win_term::screen_buffer_size()?;
        let terminal_width = usize::try_from(buffer_width).unwrap_or(80).max(1);
        let column_width = completion_column_width(&menu.completions);
        let columns = usize::min(MAX_COLUMNS, usize::max(1, terminal_width / column_width));
        let rows = menu.completions.candidates.len().div_ceil(columns);
        let required_lines = i16::try_from(rows).unwrap_or(i16::MAX);

        if completions_start_y.saturating_add(required_lines) >= buffer_height {
            return self.handle_multiple_completions_fallback();
        }

        let max_rendered = rows.max(menu.rendered_lines);
        for row in 0..max_rendered {
            let y = completions_start_y + i16::try_from(row).unwrap_or(i16::MAX);
            win_term::set_cursor_position(0, y)?;
            win_term::clear_current_line()?;
        }

        for row in 0..rows {
            let y = completions_start_y + i16::try_from(row).unwrap_or(i16::MAX);
            win_term::set_cursor_position(0, y)?;

            for column in 0..columns {
                let index = column * rows + row;
                let Some(candidate) = menu.completions.candidates.get(index) else {
                    continue;
                };

                let formatted =
                    format_completion_candidate(candidate.as_str(), &menu.completions.options);
                let selected = index == menu.selected;
                let display_value = if selected {
                    formatted.to_ascii_uppercase()
                } else {
                    formatted
                };
                let marker = if selected { '>' } else { ' ' };
                let cell = format_completion_cell(marker, &display_value, column_width);
                eprint!("{cell}");
            }
        }

        menu.rendered_lines = rows;
        win_term::set_cursor_position(input_cursor.x, input_cursor.y)?;
        std::io::stderr().flush()?;

        Ok(())
    }

    fn handle_multiple_completions_fallback(&self) -> Result<(), ShellError> {
        let Some(menu) = &self.completion_menu else {
            return Ok(());
        };

        Self::display_newline()?;
        for (index, candidate) in menu.completions.candidates.iter().enumerate() {
            let formatted =
                format_completion_candidate(candidate.as_str(), &menu.completions.options);
            let selected = index == menu.selected;
            let display_value = if selected {
                formatted.to_ascii_uppercase()
            } else {
                formatted
            };
            let marker = if selected { '>' } else { ' ' };
            eprintln!("{marker}{display_value}");
        }
        std::io::stderr().flush()?;

        self.display_prompt()?;
        eprint!(
            "{}{}",
            self.line,
            repeated_char_str(BACKSPACE, self.line.len() - self.cursor)
        );
        std::io::stderr().flush()?;

        Ok(())
    }

    fn clear_completion_menu(&mut self) -> Result<(), ShellError> {
        let Some(menu) = self.completion_menu.take() else {
            return Ok(());
        };

        let input_cursor = win_term::get_cursor_position()?;
        let start_y = input_cursor.y.saturating_add(1);

        for row in 0..menu.rendered_lines {
            let y = start_y + i16::try_from(row).unwrap_or(i16::MAX);
            win_term::set_cursor_position(0, y)?;
            win_term::clear_current_line()?;
        }

        win_term::set_cursor_position(input_cursor.x, input_cursor.y)?;
        std::io::stderr().flush()?;
        Ok(())
    }

    fn accept_completion_selection(&mut self) -> Result<(), ShellError> {
        let Some(menu) = &self.completion_menu else {
            return Ok(());
        };
        let Some(candidate) = menu.completions.candidates.get(menu.selected).cloned() else {
            return Ok(());
        };
        let insertion_index = menu.completions.insertion_index;
        let delete_count = menu.completions.delete_count;

        self.clear_completion_menu()?;
        self.apply_completion(insertion_index, delete_count, candidate)
    }

    #[expect(
        clippy::string_slice,
        reason = "all offsets are expected to be at char boundaries"
    )]
    fn apply_completion(
        &mut self,
        insertion_index: usize,
        delete_count: usize,
        candidate: String,
    ) -> Result<(), ShellError> {
        if insertion_index + delete_count != self.cursor {
            return Ok(());
        }

        let mut updated_line = self.line.clone();
        updated_line.truncate(insertion_index);
        updated_line.push_str(&candidate);
        updated_line.push_str(&self.line[self.cursor..]);
        self.line = updated_line;
        self.cursor = insertion_index + candidate.len();

        win_term::clear_current_line()?;
        self.display_prompt()?;
        eprint!(
            "{}{}",
            self.line,
            repeated_char_str(BACKSPACE, self.line.len() - self.cursor)
        );
        std::io::stderr().flush()?;
        Ok(())
    }
}

#[allow(clippy::string_slice)]
fn format_completion_candidate(
    mut candidate: &str,
    options: &crate::core::completion::ProcessingOptions,
) -> String {
    if options.treat_as_filenames {
        let trimmed = crate::core::sys::fs::strip_path_separator_suffix(candidate);
        if let Some(index) = crate::core::sys::fs::rfind_path_separator(trimmed) {
            candidate = &candidate[index + 1..];
        }
    }

    candidate.to_string()
}

fn completion_column_width(completions: &crate::core::completion::Completions) -> usize {
    let max_candidate_width = completions
        .candidates
        .iter()
        .map(|candidate| {
            format_completion_candidate(candidate.as_str(), &completions.options)
                .chars()
                .count()
        })
        .max()
        .unwrap_or(1);

    max_candidate_width + 2
}

fn format_completion_cell(marker: char, value: &str, width: usize) -> String {
    let available = width.saturating_sub(2);
    let mut value = value.chars().take(available).collect::<String>();
    if value.chars().count() < available {
        value.push_str(&" ".repeat(available - value.chars().count()));
    }
    format!("{marker}{value} ")
}

fn repeated_char_str(c: char, count: usize) -> String {
    (0..count).map(|_| c).collect()
}
