use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

use super::builtin;
use super::{Error, LogicalOperator, Pipeline, Program, RedirectKind, SimpleCommand, Word};

/// `besh` 的可变运行状态.
#[derive(Clone, Debug)]
pub struct Shell {
    pub(super) cwd: PathBuf,
    pub(super) env: HashMap<String, String>,
    pub(super) last_status: i32,
    pub(super) exit_code: Option<i32>,
}

impl Shell {
    /// 从当前进程环境创建 Shell.
    pub fn new() -> Result<Self, Error> {
        let cwd = std::env::current_dir()?;
        let mut env: HashMap<String, String> = std::env::vars().collect();
        env.insert("PWD".into(), cwd.to_string_lossy().into_owned());
        Ok(Self {
            cwd,
            env,
            last_status: 0,
            exit_code: None,
        })
    }

    /// 解析并执行一段输入.
    pub fn run(&mut self, input: &str) -> Result<i32, Error> {
        let program = super::parse(input)?;
        self.execute(program)
    }

    /// 执行已解析程序.
    pub fn execute(&mut self, program: Program) -> Result<i32, Error> {
        for chain in program.chains {
            let mut status = self.execute_pipeline(chain.first)?;
            self.last_status = status;

            for (operator, pipeline) in chain.rest {
                let should_run = match operator {
                    LogicalOperator::And => status == 0,
                    LogicalOperator::Or => status != 0,
                };
                if should_run {
                    status = self.execute_pipeline(pipeline)?;
                    self.last_status = status;
                }
                if self.exit_code.is_some() {
                    break;
                }
            }
            if self.exit_code.is_some() {
                break;
            }
        }
        Ok(self.exit_code.unwrap_or(self.last_status))
    }

    /// 返回 builtin `exit` 是否请求退出.
    pub const fn should_exit(&self) -> bool {
        self.exit_code.is_some()
    }

    /// 返回当前工作目录.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub(super) fn absolute_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.cwd.join(path)
        }
    }

    fn execute_pipeline(&mut self, pipeline: Pipeline) -> Result<i32, Error> {
        if pipeline.commands.len() == 1 {
            return self.execute_single(pipeline.commands.into_iter().next().unwrap());
        }
        self.execute_multi_stage(pipeline.commands)
    }

    fn execute_single(&mut self, command: SimpleCommand) -> Result<i32, Error> {
        let ExpandedCommand {
            assignments,
            words,
            redirects,
        } = self.expand_command(command);

        if words.is_empty() {
            for (name, value) in assignments {
                self.env.insert(name, value);
            }
            prepare_redirects(self, &redirects)?;
            return Ok(0);
        }

        let name = &words[0];
        let args = &words[1..];
        if builtin::is_builtin(name) {
            validate_input_redirects(self, &redirects)?;
            let saved = apply_assignments(&mut self.env, &assignments);
            let mut output = output_writer(self, &redirects, None)?;
            let result = builtin::run(self, name, args, &mut output);
            restore_assignments(&mut self.env, saved);
            result
        } else {
            let mut process = external_command(self, name, args, &assignments);
            apply_external_redirects(self, &mut process, &redirects, None, None)?;
            let status = process
                .status()
                .map_err(|error| Error::Message(format!("{name}: 无法执行: {error}")))?;
            Ok(status.code().unwrap_or(1))
        }
    }

    fn execute_multi_stage(&mut self, commands: Vec<SimpleCommand>) -> Result<i32, Error> {
        let count = commands.len();
        let mut readers = Vec::with_capacity(count - 1);
        let mut writers = Vec::with_capacity(count - 1);
        for _ in 0..count - 1 {
            let (reader, writer) = std::io::pipe()?;
            readers.push(Some(reader));
            writers.push(Some(writer));
        }

        let mut stages = Vec::with_capacity(count);
        for (index, command) in commands.into_iter().enumerate() {
            let ExpandedCommand {
                assignments,
                words,
                redirects,
            } = self.expand_command(command);
            let input = (index > 0).then(|| readers[index - 1].take().unwrap());
            let output = (index + 1 < count).then(|| writers[index].take().unwrap());

            if words.is_empty() {
                prepare_redirects(self, &redirects)?;
                stages.push(RunningStage::Immediate(0));
                continue;
            }

            let name = words[0].clone();
            let args = words[1..].to_vec();
            if builtin::is_builtin(&name) {
                validate_input_redirects(self, &redirects)?;
                let mut subshell = self.clone();
                for (key, value) in assignments {
                    subshell.env.insert(key, value);
                }
                drop(input);
                let mut writer = output_writer(&subshell, &redirects, output)?;
                stages.push(RunningStage::Builtin(std::thread::spawn(move || {
                    builtin::run(&mut subshell, &name, &args, &mut writer).unwrap_or_else(|error| {
                        eprintln!("besh: {error}");
                        1
                    })
                })));
            } else {
                let mut process = external_command(self, &name, &args, &assignments);
                apply_external_redirects(self, &mut process, &redirects, input, output)?;
                match process.spawn() {
                    Ok(child) => stages.push(RunningStage::External(child)),
                    Err(error) => {
                        for stage in &mut stages {
                            stage.cancel();
                        }
                        return Err(Error::Message(format!("{name}: 无法执行: {error}")));
                    }
                }
            }
        }

        let mut last = 0;
        for stage in stages {
            last = stage.wait();
        }
        Ok(last)
    }

    fn expand_command(&self, command: SimpleCommand) -> ExpandedCommand {
        let mut words: Vec<String> = command
            .words
            .iter()
            .map(|word| self.expand_word(word))
            .collect();
        if let Some(first) = words.first_mut()
            && (first == "~" || first.starts_with("~/"))
            && let Some(home) = self.env.get("HOME")
        {
            *first = format!("{home}{}", &first[1..]);
        }

        let assignment_count = words
            .iter()
            .take_while(|word| builtin::split_assignment(word).is_some())
            .count();
        let assignments = words[..assignment_count]
            .iter()
            .filter_map(|word| builtin::split_assignment(word))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        words.drain(..assignment_count);

        let redirects = command
            .redirects
            .into_iter()
            .map(|redirect| ExpandedRedirect {
                kind: redirect.kind,
                target: self.expand_word(&redirect.target),
            })
            .collect();

        ExpandedCommand {
            assignments,
            words,
            redirects,
        }
    }

    fn expand_word(&self, word: &Word) -> String {
        let mut result = String::new();
        for segment in &word.segments {
            if segment.expand {
                expand_variables(self, &segment.text, &mut result);
            } else {
                result.push_str(&segment.text);
            }
        }
        result
    }
}

struct ExpandedCommand {
    assignments: Vec<(String, String)>,
    words: Vec<String>,
    redirects: Vec<ExpandedRedirect>,
}

struct ExpandedRedirect {
    kind: RedirectKind,
    target: String,
}

enum RunningStage {
    External(Child),
    Builtin(JoinHandle<i32>),
    Immediate(i32),
}

impl RunningStage {
    fn wait(self) -> i32 {
        match self {
            Self::External(mut child) => child
                .wait()
                .ok()
                .and_then(|status| status.code())
                .unwrap_or(1),
            Self::Builtin(task) => task.join().unwrap_or(1),
            Self::Immediate(status) => status,
        }
    }

    fn cancel(&mut self) {
        if let Self::External(child) = self {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

enum StageWriter {
    Stdout(std::io::Stdout),
    File(std::fs::File),
    Pipe(std::io::PipeWriter),
}

impl Write for StageWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Stdout(output) => output.write(buffer),
            Self::File(output) => output.write(buffer),
            Self::Pipe(output) => output.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Stdout(output) => output.flush(),
            Self::File(output) => output.flush(),
            Self::Pipe(output) => output.flush(),
        }
    }
}

fn external_command(
    shell: &Shell,
    name: &str,
    args: &[String],
    assignments: &[(String, String)],
) -> Command {
    let mut command = Command::new(name);
    command
        .current_dir(&shell.cwd)
        .args(args)
        .env_clear()
        .envs(&shell.env);
    for (key, value) in assignments {
        command.env(key, value);
    }
    command
}

fn prepare_redirects(shell: &Shell, redirects: &[ExpandedRedirect]) -> Result<(), Error> {
    let _ = output_writer(shell, redirects, None)?;
    validate_input_redirects(shell, redirects)
}

fn validate_input_redirects(shell: &Shell, redirects: &[ExpandedRedirect]) -> Result<(), Error> {
    for redirect in redirects {
        if matches!(redirect.kind, RedirectKind::Input) {
            let _ = std::fs::File::open(shell.absolute_path(Path::new(&redirect.target)))?;
        }
    }
    Ok(())
}

fn output_writer(
    shell: &Shell,
    redirects: &[ExpandedRedirect],
    pipeline: Option<std::io::PipeWriter>,
) -> Result<StageWriter, Error> {
    let mut writer = pipeline.map(StageWriter::Pipe);
    for redirect in redirects {
        match redirect.kind {
            RedirectKind::Input => {}
            RedirectKind::Output | RedirectKind::Append => {
                let mut options = std::fs::OpenOptions::new();
                options.create(true).write(true);
                if matches!(redirect.kind, RedirectKind::Append) {
                    options.append(true);
                } else {
                    options.truncate(true);
                }
                writer = Some(StageWriter::File(
                    options.open(shell.absolute_path(Path::new(&redirect.target)))?,
                ));
            }
        }
    }
    Ok(writer.unwrap_or_else(|| StageWriter::Stdout(std::io::stdout())))
}

fn apply_external_redirects(
    shell: &Shell,
    command: &mut Command,
    redirects: &[ExpandedRedirect],
    pipeline_input: Option<std::io::PipeReader>,
    pipeline_output: Option<std::io::PipeWriter>,
) -> Result<(), Error> {
    if let Some(input) = pipeline_input {
        command.stdin(Stdio::from(input));
    }
    if let Some(output) = pipeline_output {
        command.stdout(Stdio::from(output));
    }
    for redirect in redirects {
        let path = shell.absolute_path(Path::new(&redirect.target));
        match redirect.kind {
            RedirectKind::Input => {
                command.stdin(Stdio::from(std::fs::File::open(path)?));
            }
            RedirectKind::Output | RedirectKind::Append => {
                let mut options = std::fs::OpenOptions::new();
                options.create(true).write(true);
                if matches!(redirect.kind, RedirectKind::Append) {
                    options.append(true);
                } else {
                    options.truncate(true);
                }
                command.stdout(Stdio::from(options.open(path)?));
            }
        }
    }
    Ok(())
}

fn apply_assignments(
    env: &mut HashMap<String, String>,
    assignments: &[(String, String)],
) -> Vec<(String, Option<String>)> {
    assignments
        .iter()
        .map(|(name, value)| {
            let previous = env.insert(name.clone(), value.clone());
            (name.clone(), previous)
        })
        .collect()
}

fn restore_assignments(env: &mut HashMap<String, String>, saved: Vec<(String, Option<String>)>) {
    for (name, value) in saved {
        if let Some(value) = value {
            env.insert(name, value);
        } else {
            env.remove(&name);
        }
    }
}

fn expand_variables(shell: &Shell, input: &str, output: &mut String) {
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '$' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        match chars.get(index + 1).copied() {
            Some('?') => {
                output.push_str(&shell.last_status.to_string());
                index += 2;
            }
            Some('$') => {
                output.push_str(&std::process::id().to_string());
                index += 2;
            }
            Some('{') => {
                let start = index + 2;
                let Some(relative_end) = chars[start..]
                    .iter()
                    .position(|character| *character == '}')
                else {
                    output.push('$');
                    index += 1;
                    continue;
                };
                let end = start + relative_end;
                let name: String = chars[start..end].iter().collect();
                if let Some(value) = shell.env.get(&name) {
                    output.push_str(value);
                }
                index = end + 1;
            }
            Some(first) if first == '_' || first.is_ascii_alphabetic() => {
                let start = index + 1;
                let mut end = start + 1;
                while chars
                    .get(end)
                    .is_some_and(|character| *character == '_' || character.is_ascii_alphanumeric())
                {
                    end += 1;
                }
                let name: String = chars[start..end].iter().collect();
                if let Some(value) = shell.env.get(&name) {
                    output.push_str(value);
                }
                index = end;
            }
            _ => {
                output.push('$');
                index += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_variables_and_status() {
        let mut shell = Shell::new().unwrap();
        shell.env.insert("BESH_TEST".into(), "value".into());
        shell.last_status = 7;
        let mut output = String::new();
        expand_variables(&shell, "$BESH_TEST $?", &mut output);
        assert_eq!(output, "value 7");
    }

    #[test]
    fn assignment_only_updates_environment() {
        let mut shell = Shell::new().unwrap();
        shell.run("BESH_TEST=updated").unwrap();
        assert_eq!(shell.env.get("BESH_TEST").unwrap(), "updated");
    }

    #[test]
    fn logical_operators_short_circuit() {
        let mut shell = Shell::new().unwrap();
        shell
            .run("false && BESH_SKIP=bad; false || BESH_SET=ok")
            .unwrap();
        assert!(!shell.env.contains_key("BESH_SKIP"));
        assert_eq!(shell.env.get("BESH_SET").unwrap(), "ok");
    }

    #[test]
    fn builtin_output_can_be_redirected() {
        let mut shell = Shell::new().unwrap();
        let path = std::env::temp_dir().join(format!("besh-redirect-{}.txt", std::process::id()));
        let command = format!("echo hello > \"{}\"", path.display());
        shell.run(&command).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\n");
        std::fs::remove_file(path).unwrap();
    }
}
