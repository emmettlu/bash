//! Definition of shell behavior traits and defaults.

use crate::engine::{Shell, error};

/// 定义 shell 错误格式化行为的 trait.
pub trait ErrorFormatter: Send + Sync + 'static {
    /// 在给定 shell 上下文中格式化错误.
    ///
    /// # Arguments
    ///
    /// * `error` - 要格式化的错误
    /// * `shell` - 发生错误的 shell 上下文
    fn format_error(&self, error: &error::Error, shell: &Shell) -> String {
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
