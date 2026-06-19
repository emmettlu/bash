#![allow(dead_code)]

use futures::lock::Mutex;
use std::sync::Arc;

pub mod builtins;
pub mod engine;
pub mod interactive;
pub mod parser;
pub mod shell;

#[derive(Debug)]
pub struct ExitCode(i32);

impl ExitCode {
    pub const fn new(code: i32) -> Self {
        Self(code)
    }

    pub const fn code(&self) -> i32 {
        self.0
    }
}

impl std::fmt::Display for ExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

impl std::error::Error for ExitCode {}

/// Main entry point for the `brush` shell.
pub async fn run() -> anyhow::Result<()> {
    let mut cli_args: Vec<_> = std::env::args().collect();

    // Work around clap's limitations handling +O options.
    for arg in &mut cli_args {
        if arg.starts_with("+O") {
            arg.insert_str(0, "--");
        }
    }

    let args = match shell::args::CommandLineArgs::try_parse_from(cli_args.iter().cloned()) {
        Ok(parsed_args) => parsed_args,
        Err(e) => {
            let _ = e.print();

            // Check for whether this is something we'd truly consider fatal. clap returns
            // errors for `--help`, `--version`, etc.
            let exit_code = match e.kind() {
                clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp => 0,
                _ => 2,
            };

            return Err(ExitCode::new(exit_code).into());
        }
    };

    if let Ok(mut event_config) = shell::entry::get_event_config().lock() {
        *event_config = Some(shell::events::TraceEventConfig::init(
            &args.enabled_debug_events,
            &args.disabled_events,
        ));
    }

    let file_config = shell::config::load_config(args.no_config, args.config_file.as_deref())
        .into_config_or_log()
        .map_err(|e| anyhow::anyhow!(e))?;

    // Instantiate an appropriately configured shell and wrap it in an `Arc`. Note that we do
    // *not* run any code in the shell yet. We'll delay loading profiles and such until after
    // we've set up everything else.
    let shell = shell::entry::instantiate_shell(&args, cli_args).await?;
    let shell = Arc::new(Mutex::new(shell));

    let default_backend = shell::entry::get_default_input_backend_type(&args);
    let selected_backend = args.input_backend.unwrap_or(default_backend);
    let ui_options = file_config.to_ui_options(&args);

    let result = match selected_backend {
        shell::args::InputBackendType::Basic => {
            let mut input_backend = interactive::BasicInputBackend;
            shell::entry::run_in_shell(&shell, args.clone(), &mut input_backend, &ui_options).await
        }
        shell::args::InputBackendType::Minimal => {
            let mut input_backend = interactive::MinimalInputBackend;
            shell::entry::run_in_shell(&shell, args.clone(), &mut input_backend, &ui_options).await
        }
    };

    let exit_code = match result {
        Ok(code) => code,
        Err(interactive::ShellError::ShellError(e)) => {
            let shell = shell.lock().await;
            let mut stderr = shell.stderr();
            let _ = shell.display_error(&mut stderr, &e);
            drop(shell);
            1
        }
        Err(err) => {
            tracing::error!("error: {err:#}");
            1
        }
    };

    if exit_code == 0 {
        Ok(())
    } else {
        Err(ExitCode::new(i32::from(exit_code)).into())
    }
}
