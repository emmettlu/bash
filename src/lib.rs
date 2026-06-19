#![allow(dead_code)]

pub mod builtins;
pub mod engine;
pub mod interactive;
pub mod parser;
pub mod shell;

/// Main entry point for the `brush` shell.
pub fn run() {
    let mut args: Vec<_> = std::env::args().collect();

    // Work around clap's limitations handling +O options.
    for arg in &mut args {
        if arg.starts_with("+O") {
            arg.insert_str(0, "--");
        }
    }

    let parsed_args = match shell::args::CommandLineArgs::try_parse_from(args.iter().cloned()) {
        Ok(parsed_args) => parsed_args,
        Err(e) => {
            let _ = e.print();

            // Check for whether this is something we'd truly consider fatal. clap returns
            // errors for `--help`, `--version`, etc.
            let exit_code = match e.kind() {
                clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp => 0,
                _ => 2,
            };

            std::process::exit(exit_code);
        }
    };

    let Ok(runtime) = compio::runtime::Runtime::new() else {
        tracing::error!("error: failed to create Compio runtime");
        std::process::exit(1);
    };

    let result = runtime.block_on(shell::entry::run_async(args, parsed_args));

    let exit_code = match result {
        Ok(code) => code,
        Err(err) => {
            tracing::error!("error: {err:#}");
            1
        }
    };

    std::process::exit(i32::from(exit_code));
}
