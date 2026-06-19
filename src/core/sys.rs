//! Platform abstraction facilities

pub(crate) mod stubs;

pub(crate) mod windows;
pub(crate) use windows as platform;

pub(crate) mod hostname;

pub(crate) mod traits;

pub use platform::fs::{self, DisplayPath};

pub use platform::async_pipe;
pub use platform::commands;
pub(crate) use platform::env;
pub use platform::fd;
pub use platform::input;
pub(crate) use platform::network;
pub use platform::poll;
pub use platform::process;
pub use platform::resource;
pub use platform::signal;
pub use platform::terminal;
pub(crate) use platform::users;

pub use platform::PlatformError;
