//! Standard builtins.

mod alias;
mod bind;
mod break_;
mod builtin_;
mod caller;
mod cd;
mod clear;
mod colon;
mod command;
mod complete;
mod continue_;
mod declare;
mod dirs;
mod dot;
mod echo;
mod enable;
mod eval;
mod exec;
mod exit;
mod export;
mod false_;
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
mod return_;
mod set;
mod shift;
mod shopt;
mod test;
mod times;
mod trap;
mod true_;
mod type_;
mod unalias;
mod unset;
mod wait;

mod builder;
mod factory;

pub use builder::ShellBuilderExt;
pub use factory::default_builtins;

/// Macro to define a struct that represents a shell built-in flag argument that can be
/// enabled or disabled by specifying an option with a leading '+' or '-' character.
///
/// # Arguments
///
/// - `$struct_name` - The identifier to be used for the struct to define.
/// - `$flag_char` - The character to use as the flag.
/// - `$desc` - The string description of the flag.
#[macro_export]
macro_rules! minus_or_plus_flag_arg {
    ($struct_name:ident, $flag_char:literal, $desc:literal) => {
        #[derive(clap::Parser)]
        pub(crate) struct $struct_name {
            #[arg(short = $flag_char, name = concat!(stringify!($struct_name), "_enable"), action = clap::ArgAction::SetTrue, help = $desc)]
            _enable: bool,
            #[arg(long = concat!("+", $flag_char), name = concat!(stringify!($struct_name), "_disable"), action = clap::ArgAction::SetTrue, hide = true)]
            _disable: bool,
        }

        impl From<$struct_name> for Option<bool> {
            fn from(value: $struct_name) -> Self {
                value.to_bool()
            }
        }

        impl $struct_name {
            #[allow(dead_code, reason = "may not be used in all macro instantiations")]
            pub const fn is_some(&self) -> bool {
                self._enable || self._disable
            }

            pub const fn to_bool(&self) -> Option<bool> {
                match (self._enable, self._disable) {
                    (true, false) => Some(true),
                    (false, true) => Some(false),
                    _ => None,
                }
            }
        }
    };
}
