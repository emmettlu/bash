#[derive(Debug, Default, Clone)]
pub(crate) struct Formatter {
    pub use_color: bool,
}

impl crate::engine::extensions::ErrorFormatter for Formatter {
    fn format_error(
        &self,
        err: &crate::engine::error::Error,
        _shell: &crate::engine::Shell<impl crate::engine::ShellExtensions>,
    ) -> String {
        let prefix = if self.use_color {
            "\x1b[31merror:\x1b[0m "
        } else {
            "error: "
        };

        std::format!("{prefix}{err:#}\n")
    }
}
