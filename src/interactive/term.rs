//! Windows Console API wrapper for terminal input/output.

use windows_sys::Win32::{
    System::Console::{
        CONSOLE_CURSOR_INFO, CONSOLE_SCREEN_BUFFER_INFO, COORD, ENABLE_ECHO_INPUT,
        ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, FillConsoleOutputCharacterW,
        GetConsoleCursorInfo, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle,
        INPUT_RECORD, ReadConsoleInputW, STD_ERROR_HANDLE, STD_INPUT_HANDLE, SetConsoleCursorInfo,
        SetConsoleCursorPosition, SetConsoleMode,
    },
    UI::Input::KeyboardAndMouse::{VK_BACK, VK_DOWN, VK_LEFT, VK_RETURN, VK_RIGHT, VK_TAB, VK_UP},
};

const KEY_EVENT: u16 = 0x0001;
const LEFT_CTRL_PRESSED: u32 = 0x0008;
const RIGHT_CTRL_PRESSED: u32 = 0x0004;

pub struct CursorVisibilityGuard {
    previous_visible: bool,
}

impl Drop for CursorVisibilityGuard {
    fn drop(&mut self) {
        let _ = set_cursor_visible(self.previous_visible);
    }
}

/// A keyboard event.
#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

/// Key code representing a physical or virtual key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Backspace,
    Left,
    Right,
    Up,
    Down,
    Tab,
}

/// Console cursor position.
#[derive(Debug, Clone, Copy)]
pub struct CursorPosition {
    pub x: i16,
    pub y: i16,
}

/// Key modifiers.
#[derive(Debug, Clone, Default)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
}

/// Reads a single keyboard event from stdin.
pub fn read_key_event() -> Result<KeyEvent, std::io::Error> {
    let handle = stdin_handle()?;

    loop {
        let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
        let mut events_read: u32 = 0;

        if unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut events_read) } == 0 {
            return Err(std::io::Error::last_os_error());
        }

        if events_read == 0 || record.EventType != KEY_EVENT {
            continue;
        }

        let key_event = unsafe { record.Event.KeyEvent };
        if key_event.bKeyDown == 0 {
            continue;
        }

        let ctrl = (key_event.dwControlKeyState & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED)) != 0;

        let code = match key_event.wVirtualKeyCode {
            VK_RETURN => KeyCode::Enter,
            VK_BACK => KeyCode::Backspace,
            VK_LEFT => KeyCode::Left,
            VK_RIGHT => KeyCode::Right,
            VK_UP => KeyCode::Up,
            VK_DOWN => KeyCode::Down,
            VK_TAB => KeyCode::Tab,
            _ => {
                let ch = unsafe { key_event.uChar.AsciiChar } as u8 as char;
                if ctrl
                    && ch.is_ascii_control()
                    && let Some(ctrl_char) =
                        control_char_from_virtual_key(key_event.wVirtualKeyCode)
                {
                    KeyCode::Char(ctrl_char)
                } else {
                    if !ctrl && ch.is_ascii_control() && ch != '\x08' {
                        continue;
                    }
                    KeyCode::Char(ch)
                }
            }
        };

        return Ok(KeyEvent {
            code,
            modifiers: KeyModifiers { ctrl, alt: false },
        });
    }
}

fn control_char_from_virtual_key(virtual_key: u16) -> Option<char> {
    let key = u8::try_from(virtual_key).ok()?;
    if key.is_ascii_uppercase() {
        Some(char::from(key.to_ascii_lowercase()))
    } else {
        None
    }
}

/// Clears the console screen.
pub fn clear_screen() -> Result<(), std::io::Error> {
    let handle = output_handle()?;

    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let console_size = info.dwSize;
    let nchars = (console_size.X as u32) * (console_size.Y as u32);
    let mut written: u32 = 0;
    let top_left = COORD { X: 0, Y: 0 };

    if unsafe { FillConsoleOutputCharacterW(handle, ' ' as u16, nchars, top_left, &mut written) }
        == 0
    {
        return Err(std::io::Error::last_os_error());
    }

    set_cursor_position(0, 0)
}

/// Returns the console screen buffer size.
pub fn screen_buffer_size() -> Result<(i16, i16), std::io::Error> {
    let handle = output_handle()?;
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };

    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok((info.dwSize.X, info.dwSize.Y))
}

/// Returns the current console cursor position.
pub fn get_cursor_position() -> Result<CursorPosition, std::io::Error> {
    let handle = output_handle()?;
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };

    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(CursorPosition {
        x: info.dwCursorPosition.X,
        y: info.dwCursorPosition.Y,
    })
}

/// Clears the current console line.
pub fn clear_current_line() -> Result<(), std::io::Error> {
    let handle = output_handle()?;
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };

    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let mut written = 0;
    let line_start = COORD {
        X: 0,
        Y: info.dwCursorPosition.Y,
    };

    if unsafe {
        FillConsoleOutputCharacterW(
            handle,
            ' ' as u16,
            info.dwSize.X as u32,
            line_start,
            &mut written,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }

    set_cursor_position(0, info.dwCursorPosition.Y)
}

/// Moves the cursor to the given position.
pub fn set_cursor_position(x: i16, y: i16) -> Result<(), std::io::Error> {
    let handle = output_handle()?;
    let coord = COORD { X: x, Y: y };

    if unsafe { SetConsoleCursorPosition(handle, coord) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

/// Enables raw mode: disables echo, line input, and Ctrl+C processing.
pub fn enable_raw_mode() -> Result<u32, std::io::Error> {
    let handle = stdin_handle()?;

    let mut original_mode: u32 = 0;
    if unsafe { GetConsoleMode(handle, &mut original_mode) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let raw_mode =
        original_mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);

    if unsafe { SetConsoleMode(handle, raw_mode) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(original_mode)
}

/// Restores the given console input mode.
pub fn restore_console_mode(mode: u32) -> Result<(), std::io::Error> {
    let handle = stdin_handle()?;

    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(())
}

pub fn hide_cursor() -> Result<CursorVisibilityGuard, std::io::Error> {
    let previous_visible = set_cursor_visible(false)?;
    Ok(CursorVisibilityGuard { previous_visible })
}

fn set_cursor_visible(visible: bool) -> Result<bool, std::io::Error> {
    let handle = output_handle()?;
    let mut cursor_info: CONSOLE_CURSOR_INFO = unsafe { std::mem::zeroed() };

    if unsafe { GetConsoleCursorInfo(handle, &mut cursor_info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    let previous_visible = cursor_info.bVisible != 0;
    cursor_info.bVisible = i32::from(visible);

    if unsafe { SetConsoleCursorInfo(handle, &cursor_info) } == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(previous_visible)
}

fn stdin_handle() -> Result<windows_sys::Win32::Foundation::HANDLE, std::io::Error> {
    let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
    if handle.is_null() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

fn output_handle() -> Result<windows_sys::Win32::Foundation::HANDLE, std::io::Error> {
    let handle = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
    if handle.is_null() {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}
