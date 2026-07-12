//! `besh` 的启动模式和交互界面.

mod args;
mod repl;

use std::io::{IsTerminal, Read};

use command::{Error, Shell};

use crate::command;

/// 解析参数并运行 `besh`.
pub fn run() -> Result<i32, Error> {
    let arguments = args::Arguments::parse(std::env::args().skip(1)).map_err(Error::Message)?;
    if arguments.help {
        print_help();
        return Ok(0);
    }
    if arguments.version {
        println!("besh {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }

    let mut shell = Shell::new()?;
    match arguments.mode {
        args::Mode::Command(command) => shell.run(&command),
        args::Mode::Script(path) => {
            let source = std::fs::read_to_string(&path)
                .map_err(|error| Error::Message(format!("{}: {error}", path.display())))?;
            shell.run(&source)
        }
        args::Mode::Stdin => {
            let mut source = String::new();
            std::io::stdin().read_to_string(&mut source)?;
            shell.run(&source)
        }
        args::Mode::Interactive => repl::run(&mut shell),
        args::Mode::Auto => {
            if std::io::stdin().is_terminal() {
                repl::run(&mut shell)
            } else {
                let mut source = String::new();
                std::io::stdin().read_to_string(&mut source)?;
                shell.run(&source)
            }
        }
    }
}

fn print_help() {
    println!(
        "besh - 一个最小 Shell\n\n用法:\n  besh\n  besh -c COMMAND\n  besh SCRIPT\n\n选项:\n  -c COMMAND  执行命令字符串\n  -i          强制交互模式\n  -s          从标准输入读取\n  -h, --help  显示帮助\n  -V, --version 显示版本"
    );
}
