//! Experimental builtins.

mod save;

use crate::core::builtins::{self, builtin};

/// Returns the set of experimental built-in commands.
pub fn experimental_builtins<SE: crate::core::extensions::ShellExtensions>()
-> std::collections::HashMap<String, builtins::Registration<SE>> {
    let mut m = std::collections::HashMap::<String, builtins::Registration<SE>>::new();

    m.insert("save".into(), builtin::<save::SaveCommand, SE>());

    m
}

/// Extension trait that simplifies adding experimental builtins to a shell builder.
pub trait ShellBuilderExt {
    /// Add experimental builtins to the shell being built.
    #[must_use]
    fn experimental_builtins(self) -> Self;
}

impl<SE: crate::core::extensions::ShellExtensions, S: crate::core::ShellBuilderState>
    ShellBuilderExt for crate::core::ShellBuilder<SE, S>
{
    fn experimental_builtins(self) -> Self {
        self.builtins(crate::experimental_builtins::experimental_builtins())
    }
}
