fn main() {
    let rt = compio::runtime::Runtime::new().unwrap();

    if let Err(error) = rt.block_on(bash::run())
        && let Some(exit_code) = error.downcast_ref::<bash::ExitCode>()
    {
        std::process::exit(exit_code.code())
    }
}
