use std::io::Write;
use std::path::Path;

use super::Error;
use super::runtime::Shell;

pub(super) fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "cd" | "echo" | "env" | "exit" | "export" | "false" | "help" | "pwd" | "true" | "unset"
    )
}

pub(super) fn run(
    shell: &mut Shell,
    name: &str,
    args: &[String],
    output: &mut dyn Write,
) -> Result<i32, Error> {
    match name {
        "cd" => cd(shell, args),
        "echo" => echo(args, output),
        "env" => env(shell, output),
        "exit" => exit(shell, args),
        "export" => export(shell, args, output),
        "false" => Ok(1),
        "help" => help(output),
        "pwd" => pwd(shell, output),
        "true" => Ok(0),
        "unset" => unset(shell, args),
        _ => Err(Error::Message(format!("未知 builtin: {name}"))),
    }
}

fn cd(shell: &mut Shell, args: &[String]) -> Result<i32, Error> {
    let target = args
        .first()
        .cloned()
        .or_else(|| shell.env.get("HOME").cloned())
        .ok_or_else(|| Error::Message("cd: HOME 未设置".into()))?;
    let target = shell.absolute_path(Path::new(&target));
    let target = std::fs::canonicalize(&target)
        .map_err(|error| Error::Message(format!("cd: {}: {error}", target.display())))?;
    if !target.is_dir() {
        return Err(Error::Message(format!(
            "cd: {}: 不是目录",
            target.display()
        )));
    }
    let old = std::mem::replace(&mut shell.cwd, target.clone());
    shell
        .env
        .insert("OLDPWD".into(), old.to_string_lossy().into_owned());
    shell
        .env
        .insert("PWD".into(), target.to_string_lossy().into_owned());
    Ok(0)
}

fn echo(args: &[String], output: &mut dyn Write) -> Result<i32, Error> {
    let (newline, args) = if args.first().is_some_and(|arg| arg == "-n") {
        (false, &args[1..])
    } else {
        (true, args)
    };
    write!(output, "{}", args.join(" "))?;
    if newline {
        writeln!(output)?;
    }
    Ok(0)
}

fn env(shell: &Shell, output: &mut dyn Write) -> Result<i32, Error> {
    let mut entries: Vec<_> = shell.env.iter().collect();
    entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
    for (name, value) in entries {
        writeln!(output, "{name}={value}")?;
    }
    Ok(0)
}

fn exit(shell: &mut Shell, args: &[String]) -> Result<i32, Error> {
    let status = args.first().map_or(Ok(shell.last_status), |value| {
        value
            .parse::<i32>()
            .map_err(|_| Error::Message(format!("exit: {value}: 需要整数")))
    })?;
    shell.exit_code = Some(status);
    Ok(status)
}

fn export(shell: &mut Shell, args: &[String], output: &mut dyn Write) -> Result<i32, Error> {
    if args.is_empty() {
        return env(shell, output);
    }
    for arg in args {
        if let Some((name, value)) = split_assignment(arg) {
            shell.env.insert(name.into(), value.into());
        } else if !is_name(arg) {
            return Err(Error::Message(format!("export: {arg}: 无效名称")));
        } else {
            shell.env.entry(arg.clone()).or_default();
        }
    }
    Ok(0)
}

fn help(output: &mut dyn Write) -> Result<i32, Error> {
    writeln!(
        output,
        "besh builtins: cd echo env exit export false help pwd true unset"
    )?;
    Ok(0)
}

fn pwd(shell: &Shell, output: &mut dyn Write) -> Result<i32, Error> {
    writeln!(output, "{}", shell.cwd.display())?;
    Ok(0)
}

fn unset(shell: &mut Shell, args: &[String]) -> Result<i32, Error> {
    for name in args {
        shell.env.remove(name);
    }
    Ok(0)
}

pub(super) fn split_assignment(value: &str) -> Option<(&str, &str)> {
    let (name, value) = value.split_once('=')?;
    is_name(name).then_some((name, value))
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}
