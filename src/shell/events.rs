//! Facilities for configuring logging events in the shell.

use std::{collections::HashSet, fmt::Display, str::FromStr};

use crate::engine::trace_categories;

/// 要记录的事件类型。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TraceEvent {
    /// 跟踪算术表达式解析和求值。
    Arithmetic,
    /// 跟踪命令执行。
    Commands,
    /// 跟踪命令补全生成。
    Complete,
    /// 跟踪单词展开。
    Expand,
    /// 跟踪函数。
    Functions,
    /// 跟踪输入控制。
    Input,
    /// 跟踪作业管理。
    Jobs,
    /// 跟踪 token 到 AST 的解析过程。
    Parse,
    /// 跟踪模式匹配。
    Pattern,
    /// 跟踪输入文本 token 化过程。
    Tokenize,
    /// 跟踪未实现功能的使用。
    Unimplemented,
}

impl Display for TraceEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Arithmetic => trace_categories::ARITHMETIC,
            Self::Commands => trace_categories::COMMANDS,
            Self::Complete => "complete",
            Self::Expand => "expand",
            Self::Functions => trace_categories::FUNCTIONS,
            Self::Input => trace_categories::INPUT,
            Self::Jobs => trace_categories::JOBS,
            Self::Parse => trace_categories::PARSE,
            Self::Pattern => trace_categories::PATTERN,
            Self::Tokenize => trace_categories::TOKENIZE,
            Self::Unimplemented => trace_categories::UNIMPLEMENTED,
        };
        f.write_str(value)
    }
}

impl FromStr for TraceEvent {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            trace_categories::ARITHMETIC => Ok(Self::Arithmetic),
            trace_categories::COMMANDS => Ok(Self::Commands),
            "complete" => Ok(Self::Complete),
            "expand" => Ok(Self::Expand),
            trace_categories::FUNCTIONS => Ok(Self::Functions),
            trace_categories::INPUT => Ok(Self::Input),
            trace_categories::JOBS => Ok(Self::Jobs),
            trace_categories::PARSE => Ok(Self::Parse),
            trace_categories::PATTERN => Ok(Self::Pattern),
            trace_categories::TOKENIZE => Ok(Self::Tokenize),
            trace_categories::UNIMPLEMENTED | "unimp" => Ok(Self::Unimplemented),
            _ => Err(format!("unknown trace event '{value}'")),
        }
    }
}

#[derive(Default)]
pub(crate) struct TraceEventConfig {
    enabled_debug_events: HashSet<TraceEvent>,
    disabled_events: HashSet<TraceEvent>,
}

impl TraceEventConfig {
    pub fn init(enabled_debug_events: &[TraceEvent], disabled_events: &[TraceEvent]) -> Self {
        let config = Self {
            enabled_debug_events: enabled_debug_events.iter().copied().collect(),
            disabled_events: disabled_events.iter().copied().collect(),
        };

        config.init_logger();
        config
    }

    fn init_logger(&self) {
        let debug_targets = self
            .enabled_debug_events
            .iter()
            .flat_map(Self::event_to_log_targets)
            .map(String::from)
            .collect::<Vec<_>>();
        let disabled_targets = self
            .disabled_events
            .iter()
            .flat_map(Self::event_to_log_targets)
            .map(String::from)
            .collect::<Vec<_>>();

        let level = if debug_targets.is_empty() {
            nanologger::LogLevel::Info
        } else {
            nanologger::LogLevel::Debug
        };

        let mut builder = nanologger::LoggerBuilder::new()
            .level(level)
            .timestamps(false)
            .source_location(false)
            .thread_info(false)
            .module_deny(disabled_targets)
            .add_output(nanologger::LogOutput::term(level));

        // nanologger only supports a global level plus module allow/deny lists.
        // When the user enables specific debug categories, restrict output to those
        // targets to avoid enabling debug logs for every module.
        if !debug_targets.is_empty() {
            builder = builder.module_allow(debug_targets);
        }

        if builder.init().is_err() {
            eprintln!("warning: failed to initialize logger.");
        }
    }

    fn event_to_log_targets(event: &TraceEvent) -> Vec<&'static str> {
        match event {
            TraceEvent::Arithmetic => vec![trace_categories::ARITHMETIC],
            TraceEvent::Commands => vec![trace_categories::COMMANDS],
            TraceEvent::Complete => vec![trace_categories::COMPLETION],
            TraceEvent::Expand => vec![trace_categories::EXPANSION],
            TraceEvent::Functions => vec![trace_categories::FUNCTIONS],
            TraceEvent::Input => vec![trace_categories::INPUT],
            TraceEvent::Jobs => vec![trace_categories::JOBS],
            TraceEvent::Parse => vec![trace_categories::PARSE],
            TraceEvent::Pattern => vec![trace_categories::PATTERN],
            TraceEvent::Tokenize => vec![trace_categories::TOKENIZE],
            TraceEvent::Unimplemented => vec![trace_categories::UNIMPLEMENTED],
        }
    }
}
