/// 承载当前进程的终端信息。
#[derive(Clone, Debug, Default)]
pub struct TerminalInfo {
    /// 检测到的终端类型。
    pub terminal: Option<KnownTerminal>,

    /// 终端分配的 session nonce。
    pub session_nonce: Option<String>,

    /// 终端是否支持 OSC 633 shell integration 序列。
    pub supports_osc_633: bool,
}

/// 标识承载当前进程的已知终端模拟器。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KnownTerminal {
    /// Alacritty。
    Alacritty,
    /// Apple Terminal。
    AppleTerminal,
    /// Ghostty。
    Ghostty,
    /// GNOME Terminal。
    GnomeTerminal,
    /// iTerm2。
    ITerm2,
    /// Kitty。
    Kitty,
    /// Konsole。
    Konsole,
    /// `VSCode` Terminal。
    VSCode,
    /// 其他基于 VTE 的终端。
    Vte,
    /// Warp Terminal。
    WarpTerminal,
    /// `WezTerm`。
    WezTerm,
    /// Windows Terminal。
    WindowsTerminal,
}

/// 抽象终端检测所需的环境变量访问。
pub(crate) trait TerminalEnvironment {
    /// 获取环境变量值, 未设置时返回 `None`。
    fn get_env_var(&self, key: &str) -> Option<String>;
}

pub(crate) fn get_terminal_info(env: &impl TerminalEnvironment) -> TerminalInfo {
    let terminal = try_detect_terminal(env);
    let mut info = TerminalInfo {
        terminal,
        ..Default::default()
    };

    match info.terminal {
        Some(KnownTerminal::VSCode) => {
            info.supports_osc_633 = true;
            info.session_nonce = env.get_env_var("VSCODE_NONCE");
        }
        Some(KnownTerminal::WindowsTerminal) => {
            info.supports_osc_633 = true;
        }
        _ => {}
    }

    info
}

/// 尝试检测承载当前进程的终端。
///
/// # Arguments
///
/// * `env` - 用于访问环境变量的 `TerminalEnvironment` 实现。
pub(crate) fn try_detect_terminal(env: &impl TerminalEnvironment) -> Option<KnownTerminal> {
    if let Some(detected) = try_detect_terminal_from_prog_var(env) {
        Some(detected)
    } else if env.get_env_var("WT_SESSION").is_some() {
        Some(KnownTerminal::WindowsTerminal)
    } else {
        None
    }
}

fn try_detect_terminal_from_prog_var(env: &impl TerminalEnvironment) -> Option<KnownTerminal> {
    let term_prog = env.get_env_var("TERM_PROGRAM")?;

    // 移除标点并归一化。
    let term_prog: String = term_prog
        .chars()
        .filter(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    match term_prog.as_str() {
        "alacritty" => Some(KnownTerminal::Alacritty),
        "appleterminal" => Some(KnownTerminal::AppleTerminal),
        "ghostty" => Some(KnownTerminal::Ghostty),
        "gnometerminal" => Some(KnownTerminal::GnomeTerminal),
        "iterm" | "iterm2" | "itermapp" => Some(KnownTerminal::ITerm2),
        "kitty" => Some(KnownTerminal::Kitty),
        "konsole" => Some(KnownTerminal::Konsole),
        "vscode" => Some(KnownTerminal::VSCode),
        "vte" => Some(KnownTerminal::Vte),
        "warp" | "warpterminal" => Some(KnownTerminal::WarpTerminal),
        "wezterm" => Some(KnownTerminal::WezTerm),
        "windowsterminal" => Some(KnownTerminal::WindowsTerminal),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_matches;
    use std::collections::HashMap;

    impl TerminalEnvironment for HashMap<&str, &str> {
        fn get_env_var(&self, key: &str) -> Option<String> {
            self.get(key).map(|v| (*v).to_string())
        }
    }

    #[test]
    fn no_term_program() {
        let test_env = HashMap::new();

        let term_info = get_terminal_info(&test_env);
        assert_matches!(term_info.terminal, None);
        assert!(!term_info.supports_osc_633);
    }

    #[test]
    fn unknown_term_program() {
        let test_env = HashMap::from([("TERM_PROGRAM", "unknown_terminal")]);

        let term_info = get_terminal_info(&test_env);
        assert_matches!(term_info.terminal, None);
        assert!(!term_info.supports_osc_633);
    }

    #[test]
    fn vscode_recognition() {
        let test_env = HashMap::from([("TERM_PROGRAM", "vscode"), ("VSCODE_NONCE", "test_nonce")]);

        let term_info = get_terminal_info(&test_env);
        assert_matches!(term_info.terminal, Some(KnownTerminal::VSCode));
        assert!(term_info.supports_osc_633);
        assert_eq!(term_info.session_nonce, Some("test_nonce".to_string()));
    }

    #[test]
    fn windows_terminal_recognition() {
        let test_env = HashMap::from([("WT_SESSION", "some_value")]);

        let term_info = get_terminal_info(&test_env);
        assert_matches!(term_info.terminal, Some(KnownTerminal::WindowsTerminal));
        assert!(term_info.supports_osc_633);
    }
}
