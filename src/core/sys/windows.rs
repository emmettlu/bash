pub use crate::core::sys::stubs::async_pipe;
pub use crate::core::sys::stubs::commands;
pub(crate) mod env;
pub use crate::core::sys::stubs::fd;
pub(crate) mod fs;
pub use crate::core::sys::stubs::input;
pub(crate) mod network;
pub use crate::core::sys::stubs::poll;
pub use crate::core::sys::stubs::resource;

/// Signal processing utilities
pub mod signal {
    pub(crate) use crate::core::sys::stubs::signal::*;
}

pub use crate::core::sys::stubs::process;
pub use crate::core::sys::stubs::terminal;
pub(crate) mod users;

/// Platform-specific errors.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {}
