use std::sync::Arc;

use futures::lock::Mutex;

/// A reference-counted, thread-safe reference to a `crate::engine::Shell`.
#[allow(type_alias_bounds)]
pub type ShellRef<
    SE: crate::engine::ShellExtensions = crate::engine::extensions::DefaultShellExtensions,
> = Arc<Mutex<crate::engine::Shell<SE>>>;
