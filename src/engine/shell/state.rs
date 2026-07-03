//! 定义 shell 状态 trait (用于 dyn 动态分发).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::engine::options::RuntimeOptions;
use crate::engine::pathcache::PathCache;

/// dyn-safe trait, 仅包含通过 `dyn ShellState` 实际调用的方法.
/// 其余 shell 状态访问方法定义为 `Shell<SE>` 的 inherent 方法.
pub trait ShellState {
    /// 返回 shell 的调用栈.
    fn call_stack(&self) -> &crate::engine::callstack::CallStack;

    /// 返回 shell 的运行时选项.
    fn options(&self) -> &RuntimeOptions;

    /// 返回当前 subshell 嵌套深度, 0 表示非 subshell.
    fn depth(&self) -> usize;

    /// 返回 shell 的历史记录 (如果存在).
    fn history(&self) -> Option<&crate::engine::history::History>;

    /// 返回上一条管道中各命令的退出状态.
    fn last_pipeline_statuses(&self) -> &[u8];

    /// 返回上次 SECONDS 计时的起始时刻.
    fn last_stopwatch_time(&self) -> std::time::SystemTime;

    /// 返回上次 SECONDS 的偏移量.
    fn last_stopwatch_offset(&self) -> u32;

    /// 返回当前 shell 名称 ($0), 受调用栈影响.
    fn current_shell_name(&self) -> Option<Cow<'_, str>>;

    /// 返回当前工作目录.
    fn working_dir(&self) -> &Path;

    /// 返回 shell 的别名表.
    fn aliases(&self) -> &HashMap<String, String>;

    /// 返回命令路径缓存.
    fn program_location_cache(&self) -> &PathCache;

    /// 返回目录栈.
    fn directory_stack(&self) -> &[PathBuf];
}
