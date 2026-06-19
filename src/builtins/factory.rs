use std::collections::HashMap;

#[allow(clippy::wildcard_imports)]
use super::*;

use crate::engine::builtins::{self, builtin, decl_builtin, raw_arg_builtin, simple_builtin};

/// Returns the default set of Bash-compatible built-in commands.
#[allow(clippy::too_many_lines)]
pub fn default_builtins<SE: crate::engine::ShellExtensions>()
-> HashMap<String, builtins::Registration<SE>> {
    let mut m = HashMap::<String, builtins::Registration<SE>>::new();

    //
    // Bash special builtins
    //

    m.insert(
        "break".into(),
        builtin::<break_::BreakCommand, SE>().special(),
    );
    m.insert(
        ":".into(),
        simple_builtin::<colon::ColonCommand, SE>().special(),
    );
    m.insert(
        "continue".into(),
        builtin::<continue_::ContinueCommand, SE>().special(),
    );
    m.insert(".".into(), builtin::<dot::DotCommand, SE>().special());
    m.insert("eval".into(), builtin::<eval::EvalCommand, SE>().special());
    m.insert("exec".into(), builtin::<exec::ExecCommand, SE>().special());
    m.insert("exit".into(), builtin::<exit::ExitCommand, SE>().special());
    m.insert(
        "export".into(),
        decl_builtin::<export::ExportCommand, SE>().special(),
    );
    m.insert(
        "return".into(),
        builtin::<return_::ReturnCommand, SE>().special(),
    );
    m.insert("set".into(), builtin::<set::SetCommand, SE>().special());
    m.insert(
        "shift".into(),
        builtin::<shift::ShiftCommand, SE>().special(),
    );
    m.insert("trap".into(), builtin::<trap::TrapCommand, SE>().special());
    m.insert(
        "unset".into(),
        builtin::<unset::UnsetCommand, SE>().special(),
    );

    m.insert(
        "readonly".into(),
        decl_builtin::<declare::DeclareCommand, SE>().special(),
    );
    m.insert(
        "times".into(),
        builtin::<times::TimesCommand, SE>().special(),
    );

    //
    // Non-special builtins
    //

    m.insert("alias".into(), builtin::<alias::AliasCommand, SE>()); // TODO(alias): should be exec_declaration_builtin
    m.insert("cd".into(), builtin::<cd::CdCommand, SE>());
    m.insert("clear".into(), builtin::<clear::ClearCommand, SE>());
    m.insert("command".into(), builtin::<command::CommandCommand, SE>());
    m.insert("false".into(), simple_builtin::<false_::FalseCommand, SE>());
    m.insert("getopts".into(), builtin::<getopts::GetOptsCommand, SE>());
    m.insert("hash".into(), builtin::<hash::HashCommand, SE>());
    m.insert("help".into(), builtin::<help::HelpCommand, SE>());
    m.insert("kill".into(), builtin::<kill::KillCommand, SE>());
    m.insert(
        "local".into(),
        decl_builtin::<declare::DeclareCommand, SE>(),
    );
    m.insert("pwd".into(), builtin::<pwd::PwdCommand, SE>());
    m.insert("read".into(), builtin::<read::ReadCommand, SE>());
    m.insert("true".into(), simple_builtin::<true_::TrueCommand, SE>());
    m.insert("type".into(), builtin::<type_::TypeCommand, SE>());
    m.insert("unalias".into(), builtin::<unalias::UnaliasCommand, SE>());
    m.insert("wait".into(), builtin::<wait::WaitCommand, SE>());

    m.insert("fc".into(), builtin::<fc::FcCommand, SE>());

    m.insert(
        "builtin".into(),
        raw_arg_builtin::<builtin_::BuiltinCommand, SE>(),
    );
    m.insert(
        "declare".into(),
        decl_builtin::<declare::DeclareCommand, SE>(),
    );
    m.insert("echo".into(), builtin::<echo::EchoCommand, SE>());
    m.insert("enable".into(), builtin::<enable::EnableCommand, SE>());
    m.insert("let".into(), builtin::<let_::LetCommand, SE>());
    m.insert("mapfile".into(), builtin::<mapfile::MapFileCommand, SE>());
    m.insert("readarray".into(), builtin::<mapfile::MapFileCommand, SE>());
    m.insert("printf".into(), builtin::<printf::PrintfCommand, SE>());
    m.insert("shopt".into(), builtin::<shopt::ShoptCommand, SE>());
    m.insert("source".into(), builtin::<dot::DotCommand, SE>().special());
    m.insert("test".into(), builtin::<test::TestCommand, SE>());
    m.insert("[".into(), builtin::<test::TestCommand, SE>());
    m.insert(
        "typeset".into(),
        decl_builtin::<declare::DeclareCommand, SE>(),
    );

    // Completion builtins
    m.insert(
        "complete".into(),
        builtin::<complete::CompleteCommand, SE>(),
    );
    m.insert("compgen".into(), builtin::<complete::CompGenCommand, SE>());
    m.insert("compopt".into(), builtin::<complete::CompOptCommand, SE>());

    // Dir stack builtins
    m.insert("dirs".into(), builtin::<dirs::DirsCommand, SE>());
    m.insert("popd".into(), builtin::<popd::PopdCommand, SE>());
    m.insert("pushd".into(), builtin::<pushd::PushdCommand, SE>());

    // Input configuration builtins
    m.insert("bind".into(), builtin::<bind::BindCommand, SE>());

    // History
    m.insert("history".into(), builtin::<history::HistoryCommand, SE>());

    m.insert("caller".into(), builtin::<caller::CallerCommand, SE>());

    m
}
