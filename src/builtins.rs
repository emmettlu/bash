//! Standard builtins.

mod alias;
mod bind;
mod builtin_;
mod caller;
mod cd;
mod command;
mod common;
mod complete;
mod declare;
mod dirs;
mod dot;
mod echo;
mod enable;
mod eval;
mod exec;
mod export;
mod fc;
mod getopts;
mod hash;
mod help;
mod history;
mod kill;
mod let_;
mod mapfile;
mod popd;
mod printf;
mod pushd;
mod pwd;
mod read;
mod set;
mod shopt;
mod small;
mod test;
mod times;
mod trap;
mod type_;
mod unalias;
mod unset;
mod wait;

mod factory;

pub use factory::default_builtins;
