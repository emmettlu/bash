/// Shell 用户界面和输入循环选项。
#[derive(Clone, bon::Builder)]
pub struct UIOptions {
    /// 这个输入循环是否代表真正的交互 session。
    #[builder(default = true)]
    pub interactive_session: bool,

    /// 是否启用终端 shell 集成。
    #[builder(default)]
    pub terminal_shell_integration: bool,

    /// 是否在 session 期间持有终端前台控制权。
    #[builder(default = true)]
    pub terminal_control: bool,

    /// 是否显示提示符。
    #[builder(default = true)]
    pub display_prompt: bool,

    /// 是否在每个提示符前运行 `PROMPT_COMMAND`。
    #[builder(default = true)]
    pub run_prompt_command: bool,

    /// 是否运行 zsh 风格的 `preexec_functions` 和 `precmd_functions`。
    #[builder(default)]
    pub run_cmd_exec_funcs: bool,
}

impl UIOptions {
    /// 返回用于 `-s` 标准输入命令循环的选项。
    #[must_use]
    pub(crate) fn stdin_input_loop() -> Self {
        Self {
            interactive_session: false,
            terminal_shell_integration: false,
            terminal_control: false,
            display_prompt: false,
            run_prompt_command: false,
            run_cmd_exec_funcs: false,
        }
    }
}
