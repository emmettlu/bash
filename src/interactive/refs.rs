use std::sync::Arc;

use futures::lock::Mutex;

/// A reference-counted, thread-safe reference to a `crate::engine::Shell`.
pub type ShellRef = Arc<Mutex<crate::engine::Shell>>;
