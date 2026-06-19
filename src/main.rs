#![allow(dead_code)]

pub mod builtins;
pub mod core;
pub mod interactive;
pub mod parser;
pub mod shell;

fn main() {
    shell::entry::run();
}
