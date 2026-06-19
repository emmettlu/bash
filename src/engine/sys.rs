//! System integration facilities for the Windows-only shell runtime.

pub(crate) mod unsupported;

pub use unsupported::async_pipe;
pub use unsupported::commands;
pub use unsupported::fd;
pub use unsupported::input;
pub use unsupported::poll;
pub use unsupported::process;
pub use unsupported::resource;
pub use unsupported::signal;
pub use unsupported::terminal;

pub(crate) mod env;
pub mod fs;
pub(crate) mod hostname;
pub(crate) mod network;
pub(crate) mod traits;
pub(crate) mod users;

/// System integration errors.
#[derive(Debug, thiserror::Error)]
pub enum SystemError {}
