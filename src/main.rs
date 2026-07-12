fn main() {
    let rt = match compio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(error) => {
            eprintln!("bash: 无法创建异步运行时: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = rt.block_on(bash::run()) {
        log::logger().flush();
        if let Some(exit_code) = error.downcast_ref::<bash::ExitCode>() {
            std::process::exit(exit_code.code());
        }

        eprintln!("bash: {error:#}");
        std::process::exit(1);
    }
}
