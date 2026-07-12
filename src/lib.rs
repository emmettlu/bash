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
            e.print()?;
            return Err(ExitCode::new(exit_code).into());
        }
    };

    let _event_config =
        shell::events::TraceEventConfig::init(&args.enabled_debug_events, &args.disabled_events);

    let plan = shell::entry::ShellRunPlan::from_args(&args);

    // Instantiate an appropriately configured shell. Note that we do *not* run any code in the
    // shell yet. We'll delay loading profiles and such until after we've set up everything else.
    let mut shell = shell::entry::instantiate_shell(&args, cli_args, &plan).await?;

    let selected_backend = args.input_backend.unwrap_or(plan.default_input_backend);
    let ui_options = interactive::UIOptions::from_args(&args, plan.interactive_session);

    let result = match selected_backend {
        shell::args::InputBackendType::Basic => {
            let mut input_backend = interactive::BasicInputBackend::default();
            shell::entry::run_in_shell(&mut shell, &args, plan, &mut input_backend, &ui_options)
                .await
        }
        shell::args::InputBackendType::Minimal => {
            let mut input_backend = interactive::MinimalInputBackend;
            shell::entry::run_in_shell(&mut shell, &args, plan, &mut input_backend, &ui_options)
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
