fn main() {
    let rt = compio::runtime::Runtime::new().unwrap();

    if let Err(error) = rt.block_on(bash::run()) {
        log::logger().flush();

        let exit_code = if let Some(exit_code) = error.downcast_ref::<bash::ExitCode>() {
            exit_code.code()
        } else {
            1
        };

        std::process::exit(exit_code);
    }
}
