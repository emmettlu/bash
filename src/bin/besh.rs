mod command;
mod ui;

fn main() {
    match ui::run() {
        Ok(status) => std::process::exit(status),
        Err(error) => {
            eprintln!("besh: {error}");
            std::process::exit(1);
        }
    }
}
