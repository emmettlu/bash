use std::sync::Arc;

use futures::lock::Mutex;

/// A reference-counted, thread-safe reference to a `crate::core::Shell`.
#[allow(type_alias_bounds)]
pub type ShellRef<
    SE: crate::core::ShellExtensions = crate::core::extensions::DefaultShellExtensions,
> = Arc<Mutex<crate::core::Shell<SE>>>;
