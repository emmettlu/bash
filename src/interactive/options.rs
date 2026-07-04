/// Options for a shell user interface.
#[derive(Default, bon::Builder)]
pub struct UIOptions {
    /// Whether to disable bracketed paste mode.
    #[allow(
        dead_code,
        reason = "配置兼容保留, 当前输入后端尚未接入 bracketed paste"
    )]
    #[builder(default)]
    pub(crate) disable_bracketed_paste: bool,
    /// Whether to disable color.
    #[allow(dead_code, reason = "配置兼容保留, 当前输入后端尚未接入彩色 UI")]
    #[builder(default)]
    pub(crate) disable_color: bool,
    /// Whether to disable syntax highlighting.
    #[allow(dead_code, reason = "配置兼容保留, 当前输入后端尚未接入语法高亮")]
    #[builder(default)]
    pub(crate) disable_highlighting: bool,
    /// Whether to enable terminal integration.
    #[builder(default)]
    pub terminal_shell_integration: bool,
    /// Whether to enable zsh-style hooks.
    #[builder(default)]
    pub zsh_style_hooks: bool,
}

impl From<&UIOptions> for crate::interactive::InteractiveOptions {
    fn from(options: &UIOptions) -> Self {
        Self {
            terminal_shell_integration: options.terminal_shell_integration,
            run_cmd_exec_funcs: options.zsh_style_hooks,
            ..Default::default()
        }
    }
}
