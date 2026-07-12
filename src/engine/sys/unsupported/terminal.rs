//! Terminal utilities.

use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::Console::{
        ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT,
        GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetConsoleMode,
    },
};

use crate::engine::{error, openfiles, sys, terminal};

/// 返回当前平台是否支持前台进程组控制.
pub const fn supports_foreground_control() -> bool {
    false
}

/// Terminal configuration.
#[derive(Clone, Debug)]
pub struct Config {
    mode: u32,
}

impl Config {
    /// Creates a new `Config` from the actual terminal attributes of the terminal associated
    /// with the given file descriptor.
    ///
    /// # Arguments
    ///
    /// * `file` - A reference to the open terminal.
    pub fn from_term(file: &openfiles::OpenFile) -> Result<Self, error::Error> {
        let handle = console_handle(file)?;
        let mut mode = 0;
        if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self { mode })
    }

    /// Applies the terminal settings to the terminal associated with the given file descriptor.
    ///
    /// # Arguments
    ///
    /// * `file` - A reference to the open terminal.
    pub fn apply_to_term(&self, file: &openfiles::OpenFile) -> Result<(), error::Error> {
        let handle = console_handle(file)?;
        if unsafe { SetConsoleMode(handle, self.mode) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }

    /// Applies the given high-level terminal settings to this configuration. Does not modify any
    /// terminal itself.
    ///
    /// # Arguments
    ///
    /// * `settings` - The high-level terminal settings to apply to this configuration.
    pub fn update(&mut self, settings: &terminal::Settings) {
        update_mode_flag(&mut self.mode, ENABLE_LINE_INPUT, settings.line_input);
        update_mode_flag(&mut self.mode, ENABLE_ECHO_INPUT, settings.echo_input);
        update_mode_flag(
            &mut self.mode,
            ENABLE_PROCESSED_INPUT,
            settings.interrupt_signals,
        );
        update_mode_flag(
            &mut self.mode,
            ENABLE_PROCESSED_OUTPUT,
            settings.output_nl_as_nlcr,
        );
        normalize_input_mode(&mut self.mode);
    }
}

fn update_mode_flag(mode: &mut u32, flag: u32, enabled: Option<bool>) {
    match enabled {
        Some(true) => *mode |= flag,
        Some(false) => *mode &= !flag,
        None => {}
    }
}

fn normalize_input_mode(mode: &mut u32) {
    if *mode & ENABLE_LINE_INPUT == 0 {
        *mode &= !ENABLE_ECHO_INPUT;
    }
}

fn console_handle(file: &openfiles::OpenFile) -> Result<HANDLE, error::Error> {
    let handle = unsafe {
        match file {
            openfiles::OpenFile::Stdin(_) => GetStdHandle(STD_INPUT_HANDLE),
            openfiles::OpenFile::Stdout(_) => GetStdHandle(STD_OUTPUT_HANDLE),
            openfiles::OpenFile::Stderr(_) => GetStdHandle(STD_ERROR_HANDLE),
            openfiles::OpenFile::File(file) => {
                use std::os::windows::io::AsRawHandle as _;
                file.as_raw_handle() as HANDLE
            }
            openfiles::OpenFile::PipeReader(_)
            | openfiles::OpenFile::PipeWriter(_)
            | openfiles::OpenFile::Stream(_) => {
                return Err(
                    error::ErrorKind::NotSupported("terminal mode for non-console stream").into(),
                );
            }
        }
    };

    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        Err(std::io::Error::last_os_error().into())
    } else {
        Ok(handle)
    }
}

/// Get the process ID of this process's parent.
///
/// This is a stub implementation that returns `None`.
pub fn get_parent_process_id() -> Option<sys::process::ProcessId> {
    None
}

/// Get the process group ID for this process's process group.
///
/// This is a stub implementation that returns `None`.
pub fn get_process_group_id() -> Option<sys::process::ProcessId> {
    None
}

/// Get the foreground process ID of the attached terminal.
///
/// This is a stub implementation that returns `None`.
pub fn get_foreground_pid() -> Option<sys::process::ProcessId> {
    None
}

/// Move the specified process to the foreground of the attached terminal.
///
/// This is a stub implementation that takes no action.
pub fn move_to_foreground(_pid: sys::process::ProcessId) -> Result<(), error::Error> {
    Ok(())
}

/// Moves the current process to the foreground of the attached terminal.
///
/// This is a stub implementation that returns `None`.
pub fn move_self_to_foreground() -> Result<(), std::io::Error> {
    Ok(())
}

/// Tries to get the path of the terminal device associated with the attached terminal.
///
/// This is a stub implementation that always returns `None`.
pub fn try_get_terminal_device_path() -> Option<std::path::PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_line_input_also_disables_echo() {
        let mut config = Config {
            mode: ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT,
        };
        let settings = terminal::Settings::builder().line_input(false).build();

        config.update(&settings);

        assert_eq!(config.mode & ENABLE_LINE_INPUT, 0);
        assert_eq!(config.mode & ENABLE_ECHO_INPUT, 0);
        assert_ne!(config.mode & ENABLE_PROCESSED_INPUT, 0);
    }

    #[test]
    fn echo_cannot_be_enabled_without_line_input() {
        let mut config = Config { mode: 0 };
        let settings = terminal::Settings::builder().echo_input(true).build();

        config.update(&settings);

        assert_eq!(config.mode & ENABLE_ECHO_INPUT, 0);
    }

    #[test]
    fn echo_can_be_enabled_with_line_input() {
        let mut config = Config { mode: 0 };
        let settings = terminal::Settings::builder()
            .line_input(true)
            .echo_input(true)
            .build();

        config.update(&settings);

        assert_ne!(config.mode & ENABLE_LINE_INPUT, 0);
        assert_ne!(config.mode & ENABLE_ECHO_INPUT, 0);
    }
}
