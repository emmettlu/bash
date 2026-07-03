//! Definition of shell behavior traits and defaults.

use crate::engine::{Shell, error, extensions};

/// Shell 行为扩展 trait, 是 `ErrorFormatter` 的 supertrait 别名.
/// 所有实现了 `ErrorFormatter` 的类型自动满足此 trait.
pub trait ShellExtensions: ErrorFormatter {}

/// 对所有 ErrorFormatter 实现提供 ShellExtensions 的自动 blanket 实现.
impl<T: ErrorFormatter> ShellExtensions for T {}

/// 默认的 shell 扩展实现, 等同于 `DefaultErrorFormatter`.
pub type DefaultShellExtensions = DefaultErrorFormatter;

/// 定义 shell 错误格式化行为的 trait.
pub trait ErrorFormatter: Clone + Default + Send + Sync + 'static {
    /// 在给定 shell 上下文中格式化错误.
    ///
    /// # Arguments
    ///
    /// * `error` - 要格式化的错误
    /// * `shell` - 发生错误的 shell 上下文
    fn format_error(
        &self,
        error: &error::Error,
        shell: &Shell<impl extensions::ShellExtensions>,
    ) -> String {
        let _ = shell;
        std::format!("error: {error:#}\n")
    }
}

/// 默认的错误格式化实现.
#[derive(Clone, Default)]
pub struct DefaultErrorFormatter;

impl ErrorFormatter for DefaultErrorFormatter {}

/// 占位行为 trait (为未来扩展保留的桩).
pub trait PlaceholderBehavior: Clone + Default + Send + Sync + 'static {}

/// 默认占位实现.
#[derive(Clone, Default)]
pub struct DefaultPlaceholder;

impl PlaceholderBehavior for DefaultPlaceholder {}
