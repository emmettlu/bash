//! File descriptor utilities.

use crate::engine::{ShellFd, openfiles};

/// Unsupported fallback for enumerating inherited file descriptors.
pub fn try_iter_open_fds() -> impl Iterator<Item = (ShellFd, openfiles::OpenFile)> {
    std::iter::empty()
}

/// Unsupported fallback for opening an inherited file descriptor.
pub fn try_get_file_for_open_fd(_fd: ShellFd) -> Option<openfiles::OpenFile> {
    None
}
