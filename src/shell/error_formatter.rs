#[derive(Debug, Default, Clone)]
pub(crate) struct Formatter {
    pub use_color: bool,
}

impl crate::core::extensions::ErrorFormatter for Formatter {
    fn format_error(
        &self,
        err: &crate::core::error::Error,
        _shell: &crate::core::Shell<impl crate::core::ShellExtensions>,
    ) -> String {
        let prefix = if self.use_color {
            color_print::cstr!("<red>error:</red> ")
        } else {
            "error: "
        };

        std::format!("{prefix}{err:#}\n")
    }
}
