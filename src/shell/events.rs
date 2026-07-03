//! Facilities for configuring logging events in the shell.

use std::{collections::HashSet, fmt::Display};

use crate::engine::Error;

/// Type of event to log.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, clap::ValueEnum)]
pub enum TraceEvent {
    /// Traces parsing and evaluation of arithmetic expressions.
    #[clap(name = "arithmetic")]
    Arithmetic,
    /// Traces command execution.
    #[clap(name = "commands")]
    Commands,
    /// Traces command completion generation.
    #[clap(name = "complete")]
    Complete,
    /// Traces word expansion.
    #[clap(name = "expand")]
    Expand,
    /// Traces functions.
    #[clap(name = "functions")]
    Functions,
    /// Traces input controls.
    #[clap(name = "input")]
    Input,
    /// Traces job management.
    #[clap(name = "jobs")]
    Jobs,
    /// Traces the process of parsing tokens into an abstract syntax tree.
    #[clap(name = "parse")]
    Parse,
    /// Traces pattern matching.
    #[clap(name = "pattern")]
    Pattern,
    /// Traces the process of tokenizing input text.
    #[clap(name = "tokenize")]
    Tokenize,
    /// Traces usage of unimplemented functionality.
    #[clap(name = "unimplemented", alias = "unimp")]
    Unimplemented,
}

impl Display for TraceEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arithmetic => write!(f, "arithmetic"),
            Self::Commands => write!(f, "commands"),
            Self::Complete => write!(f, "complete"),
            Self::Expand => write!(f, "expand"),
            Self::Functions => write!(f, "functions"),
            Self::Input => write!(f, "input"),
            Self::Jobs => write!(f, "jobs"),
            Self::Parse => write!(f, "parse"),
            Self::Pattern => write!(f, "pattern"),
            Self::Tokenize => write!(f, "tokenize"),
            Self::Unimplemented => write!(f, "unimplemented"),
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
            TraceEvent::Arithmetic => vec!["arithmetic"],
            TraceEvent::Commands => vec!["commands"],
            TraceEvent::Complete => vec!["completion"],
            TraceEvent::Expand => vec!["expansion"],
            TraceEvent::Functions => vec!["functions"],
            TraceEvent::Input => vec!["input"],
            TraceEvent::Jobs => vec!["jobs"],
            TraceEvent::Parse => vec!["parse"],
            TraceEvent::Pattern => vec!["pattern"],
            TraceEvent::Tokenize => vec!["tokenize"],
            TraceEvent::Unimplemented => vec!["unimplemented"],
        }
    }

    pub const fn get_enabled_events(&self) -> &HashSet<TraceEvent> {
        &self.enabled_debug_events
    }

    pub fn enable(&mut self, event: TraceEvent) -> Result<(), Error> {
        self.enabled_debug_events.insert(event);
        Ok(())
    }

    pub fn disable(&mut self, event: TraceEvent) -> Result<(), Error> {
        self.enabled_debug_events.remove(&event);
        self.disabled_events.insert(event);
        Ok(())
    }
}
