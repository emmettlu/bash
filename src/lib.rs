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

/// Main entry point for the shell.
pub async fn run() -> anyhow::Result<()> {
    let cli_args: Vec<_> = std::env::args().collect();

    let args = match shell::args::CommandLineArgs::try_parse_from(cli_args.iter().cloned()) {
        Ok(parsed_args) => parsed_args,
        Err(e) => {
            let exit_code = e.exit_code();
            let _ = e.print();
            return Err(ExitCode::new(exit_code).into());
        }
    };

    let _event_config =
        shell::events::TraceEventConfig::init(&args.enabled_debug_events, &args.disabled_events);

    let file_config = shell::config::load_config(args.no_config, args.config_file.as_deref())
        .into_config_or_log()
        .map_err(|e| anyhow::anyhow!(e))?;

    // Instantiate an appropriately configured shell. Note that we do *not* run any code in the
    // shell yet. We'll delay loading profiles and such until after we've set up everything else.
    let mut shell = shell::entry::instantiate_shell(&args, cli_args).await?;

    let default_backend = shell::entry::get_default_input_backend_type(&args);
    let selected_backend = args.input_backend.unwrap_or(default_backend);
    let ui_options = file_config.to_ui_options(&args);

    let result = match selected_backend {
        shell::args::InputBackendType::Basic => {
            let mut input_backend = interactive::BasicInputBackend::default();
            shell::entry::run_in_shell(&mut shell, args.clone(), &mut input_backend, &ui_options)
                .await
        }
        shell::args::InputBackendType::Minimal => {
            let mut input_backend = interactive::MinimalInputBackend;
            shell::entry::run_in_shell(&mut shell, args.clone(), &mut input_backend, &ui_options)
                .await
        }
    };

    let exit_code = match result {
        Ok(code) => code,
        Err(interactive::ShellError::ShellError(e)) => {
            let mut stderr = shell.stderr();
            let _ = shell.display_error(&mut stderr, &e);
            1
        }
        Err(err) => {
            log::error!("error: {err:#}");
            1
        }
    };

    if exit_code == 0 {
        Ok(())
    } else {
        Err(ExitCode::new(i32::from(exit_code)).into())
    }
}
