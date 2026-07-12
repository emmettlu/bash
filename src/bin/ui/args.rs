use std::path::PathBuf;

pub(super) struct Arguments {
    pub mode: Mode,
    pub help: bool,
    pub version: bool,
}

pub(super) enum Mode {
    Auto,
    Command(String),
    Script(PathBuf),
    Stdin,
    Interactive,
}

impl Arguments {
    pub(super) fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut arguments = arguments.into_iter();
        let mut mode = Mode::Auto;
        let mut help = false;
        let mut version = false;

        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "-c" => {
                    mode = Mode::Command(
                        arguments
                            .next()
                            .ok_or_else(|| "-c 后缺少命令字符串".to_owned())?,
                    );
                    break;
                }
                "-i" => mode = Mode::Interactive,
                "-s" => mode = Mode::Stdin,
                "-h" | "--help" => help = true,
                "-V" | "--version" => version = true,
                "--" => {
                    if let Some(path) = arguments.next() {
                        mode = Mode::Script(path.into());
                    }
                    break;
                }
                value if value.starts_with('-') => {
                    return Err(format!("未知选项: {value}"));
                }
                path => {
                    mode = Mode::Script(path.into());
                    break;
                }
            }
        }

        Ok(Self {
            mode,
            help,
            version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_mode() {
        let arguments = Arguments::parse(["-c".into(), "echo ok".into()]).unwrap();
        assert!(matches!(arguments.mode, Mode::Command(_)));
    }
}
