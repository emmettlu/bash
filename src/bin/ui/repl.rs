use std::io::Write;

use crate::command::{Error, Shell};

pub(super) fn run(shell: &mut Shell) -> Result<i32, Error> {
    let stdin = std::io::stdin();
    let mut buffer = String::new();

    loop {
        if buffer.is_empty() {
            print!("besh:{}$ ", shell.cwd().display());
        } else {
            print!("> ");
        }
        std::io::stdout().flush()?;

        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            if buffer.is_empty() {
                println!();
                return Ok(0);
            }
            return match shell.run(&buffer) {
                Ok(status) => Ok(status),
                Err(error) => {
                    eprintln!("besh: {error}");
                    Ok(2)
                }
            };
        }
        buffer.push_str(&line);

        match shell.run(&buffer) {
            Ok(status) => {
                buffer.clear();
                if shell.should_exit() {
                    return Ok(status);
                }
            }
            Err(error) if error.is_incomplete() => {}
            Err(error) => {
                eprintln!("besh: {error}");
                buffer.clear();
            }
        }
    }
}
