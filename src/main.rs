#![allow(dead_code)]

pub mod builtins;
pub mod core;
pub mod interactive;
pub mod parser;
pub mod shell;

#[cfg(feature = "experimental-builtins")]
pub mod experimental_builtins;

fn main() {
    shell::entry::run();
}
