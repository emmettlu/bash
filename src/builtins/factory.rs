use std::collections::HashMap;

#[allow(clippy::wildcard_imports)]
use super::*;

use crate::engine::builtins::{self, builtin, decl_builtin, raw_arg_builtin, simple_builtin};

/// Returns the default set of Bash-compatible built-in commands.
#[allow(clippy::too_many_lines)]
pub fn default_builtins() -> HashMap<String, builtins::Registration> {
    let mut m = HashMap::<String, builtins::Registration>::new();

    //
    // Bash special builtins
    //

    m.insert("break".into(), builtin::<small::BreakCommand>().special());
    m.insert(
        ":".into(),
        simple_builtin::<small::ColonCommand>().special(),
    );
    m.insert(
        "continue".into(),
        builtin::<small::ContinueCommand>().special(),
    );
    m.insert(".".into(), builtin::<dot::DotCommand>().special());
    m.insert("eval".into(), builtin::<eval::EvalCommand>().special());
    m.insert("exec".into(), builtin::<exec::ExecCommand>().special());
    m.insert("exit".into(), builtin::<small::ExitCommand>().special());
    m.insert(
        "export".into(),
        decl_builtin::<export::ExportCommand>().special(),
    );
    m.insert("return".into(), builtin::<small::ReturnCommand>().special());
    m.insert("set".into(), builtin::<set::SetCommand>().special());
    m.insert("shift".into(), builtin::<small::ShiftCommand>().special());
    m.insert("trap".into(), builtin::<trap::TrapCommand>().special());
    m.insert("unset".into(), builtin::<unset::UnsetCommand>().special());

    m.insert(
        "readonly".into(),
        decl_builtin::<declare::DeclareCommand>().special(),
    );
    m.insert("times".into(), builtin::<times::TimesCommand>().special());

    //
    // Non-special builtins
    //

    m.insert("alias".into(), builtin::<alias::AliasCommand>()); // TODO(alias): should be exec_declaration_builtin
    m.insert("cd".into(), builtin::<cd::CdCommand>());
    m.insert("clear".into(), builtin::<small::ClearCommand>());
    m.insert("command".into(), builtin::<command::CommandCommand>());
    m.insert("false".into(), simple_builtin::<small::FalseCommand>());
    m.insert("getopts".into(), builtin::<getopts::GetOptsCommand>());
    m.insert("hash".into(), builtin::<hash::HashCommand>());
    m.insert("help".into(), builtin::<help::HelpCommand>());
    m.insert("kill".into(), builtin::<kill::KillCommand>());
    m.insert("local".into(), decl_builtin::<declare::DeclareCommand>());
    m.insert("pwd".into(), builtin::<pwd::PwdCommand>());
    m.insert("read".into(), builtin::<read::ReadCommand>());
    m.insert("true".into(), simple_builtin::<small::TrueCommand>());
    m.insert("type".into(), builtin::<type_::TypeCommand>());
    m.insert("unalias".into(), builtin::<unalias::UnaliasCommand>());
    m.insert("wait".into(), builtin::<wait::WaitCommand>());

    m.insert("fc".into(), builtin::<fc::FcCommand>());

    m.insert(
        "builtin".into(),
        raw_arg_builtin::<builtin_::BuiltinCommand>(),
    );
    m.insert("declare".into(), decl_builtin::<declare::DeclareCommand>());
    m.insert("echo".into(), builtin::<echo::EchoCommand>());
    m.insert("enable".into(), builtin::<enable::EnableCommand>());
    m.insert("let".into(), builtin::<let_::LetCommand>());
    m.insert("mapfile".into(), builtin::<mapfile::MapFileCommand>());
    m.insert("readarray".into(), builtin::<mapfile::MapFileCommand>());
    m.insert("printf".into(), builtin::<printf::PrintfCommand>());
    m.insert("shopt".into(), builtin::<shopt::ShoptCommand>());
    m.insert("source".into(), builtin::<dot::DotCommand>().special());
    m.insert("test".into(), builtin::<test::TestCommand>());
    m.insert("[".into(), builtin::<test::TestCommand>());
    m.insert("typeset".into(), decl_builtin::<declare::DeclareCommand>());

    // Completion builtins
    m.insert("complete".into(), builtin::<complete::CompleteCommand>());
    m.insert("compgen".into(), builtin::<complete::CompGenCommand>());
    m.insert("compopt".into(), builtin::<complete::CompOptCommand>());

    // Dir stack builtins
    m.insert("dirs".into(), builtin::<dirs::DirsCommand>());
    m.insert("popd".into(), builtin::<popd::PopdCommand>());
    m.insert("pushd".into(), builtin::<pushd::PushdCommand>());

    // Input configuration builtins
    m.insert("bind".into(), builtin::<bind::BindCommand>());

    // History
    m.insert("history".into(), builtin::<history::HistoryCommand>());

    m.insert("caller".into(), builtin::<caller::CallerCommand>());

    m
}
