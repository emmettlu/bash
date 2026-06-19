/// Extension trait that simplifies adding default builtins to a shell builder.
pub trait ShellBuilderExt {
    /// Add default Bash-compatible builtins to the shell being built.
    #[must_use]
    fn default_builtins(self) -> Self;
}

impl<SE: crate::engine::extensions::ShellExtensions, S: crate::engine::ShellBuilderState>
    ShellBuilderExt for crate::engine::ShellBuilder<SE, S>
{
    fn default_builtins(self) -> Self {
        self.builtins(crate::builtins::default_builtins())
    }
}
