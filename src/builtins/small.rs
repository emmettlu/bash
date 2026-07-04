use std::io::Write;

use crate::engine::{ExecutionControlFlow, ExecutionResult, builtins};

/// 跳出控制流循环。
pub(crate) struct BreakCommand {
    /// 指定要跳出的嵌套循环层级。
    which_loop: i8,
}

impl builtins::Command for BreakCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let args = builtins::BuiltinArgs::new(args).rest();
        match args.as_slice() {
            [] => Ok(Self { which_loop: 1 }),
            [which_loop] => Ok(Self {
                which_loop: which_loop
                    .parse()
                    .map_err(|_| format!("break: {which_loop}: numeric argument required"))?,
            }),
            _ => Err("break: too many arguments".into()),
        }
    }

    async fn execute(
        &self,
        _context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        if self.which_loop <= 0 {
            return Ok(ExecutionResult::invalid_usage());
        }

        let mut result = ExecutionResult::success();

        result.next_control_flow = ExecutionControlFlow::BreakLoop {
            #[expect(clippy::cast_sign_loss)]
            levels: (self.which_loop - 1) as usize,
        };

        Ok(result)
    }
}

/// 清空终端屏幕。
pub(crate) struct ClearCommand {}

impl builtins::Command for ClearCommand {
    type Error = crate::engine::Error;

    fn new<I>(_args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {})
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        write!(context.stdout(), "\x1B[2J\x1B[H")?;
        context.stdout().flush()?;
        Ok(ExecutionResult::success())
    }
}

/// 空命令。
pub(crate) struct ColonCommand {}

impl builtins::SimpleCommand for ColonCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, crate::engine::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => {
                Ok("Null command; always returns success.".into())
            }
            builtins::ContentType::ShortUsage => Ok(":: :".into()),
            builtins::ContentType::ShortDescription => Ok(": - Null command".into()),
        }
    }

    fn execute<I: Iterator<Item = S>, S: AsRef<str>>(
        _context: crate::engine::ExecutionContext<'_>,
        _args: I,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        Ok(ExecutionResult::success())
    }
}

/// 继续执行控制流循环的下一次迭代。
pub(crate) struct ContinueCommand {
    /// 指定要继续执行的嵌套循环层级。
    which_loop: i8,
}

impl builtins::Command for ContinueCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let args = builtins::BuiltinArgs::new(args).rest();
        match args.as_slice() {
            [] => Ok(Self { which_loop: 1 }),
            [which_loop] => Ok(Self {
                which_loop: which_loop
                    .parse()
                    .map_err(|_| format!("continue: {which_loop}: numeric argument required"))?,
            }),
            _ => Err("continue: too many arguments".into()),
        }
    }

    async fn execute(
        &self,
        _context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        if self.which_loop <= 0 {
            return Ok(ExecutionResult::invalid_usage());
        }

        let mut result = ExecutionResult::success();

        result.next_control_flow = ExecutionControlFlow::ContinueLoop {
            #[expect(clippy::cast_sign_loss)]
            levels: (self.which_loop - 1) as usize,
        };

        Ok(result)
    }
}

/// 退出 shell。
pub(crate) struct ExitCommand {
    /// 返回的退出码。
    code: Option<i64>,
}

impl builtins::Command for ExitCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args).rest();
        if args.first().is_some_and(|arg| arg == "--") {
            args.remove(0);
        }

        match args.as_slice() {
            [] => Ok(Self { code: None }),
            [code] => Ok(Self {
                code: Some(
                    code.parse()
                        .map_err(|_| format!("exit: {code}: numeric argument required"))?,
                ),
            }),
            _ => Err("exit: too many arguments".into()),
        }
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        #[expect(clippy::cast_sign_loss)]
        let code_8bit = if let Some(code_32bit) = &self.code {
            (code_32bit & 0xFF) as u8
        } else {
            context.shell.last_exit_status()
        };

        let mut result = ExecutionResult::new(code_8bit);
        result.next_control_flow = ExecutionControlFlow::ExitShell;

        Ok(result)
    }
}

/// 返回失败退出码。
pub(crate) struct FalseCommand {}

impl builtins::Command for FalseCommand {
    type Error = crate::engine::Error;

    fn new<I>(_args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        Ok(Self {})
    }

    async fn execute(
        &self,
        _context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        Ok(ExecutionResult::general_error())
    }
}

impl builtins::SimpleCommand for FalseCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, crate::engine::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Returns a failure exit status.".into()),
            builtins::ContentType::ShortUsage => Ok("false".into()),
            builtins::ContentType::ShortDescription => Ok("false - fail".into()),
        }
    }

    fn execute<I: Iterator<Item = S>, S: AsRef<str>>(
        _context: crate::engine::ExecutionContext<'_>,
        _args: I,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        Ok(ExecutionResult::general_error())
    }
}

/// 从当前函数或 sourced 脚本返回。
pub(crate) struct ReturnCommand {
    /// 返回的退出码。
    code: Option<i32>,
}

impl builtins::Command for ReturnCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let code = match args.next_arg() {
            Some(arg) => Some(
                arg.parse()
                    .map_err(|_| format!("return: {arg}: numeric argument required"))?,
            ),
            None => None,
        };

        if args.next_arg().is_some() {
            return Err("return: too many arguments".into());
        }

        Ok(Self { code })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        #[expect(clippy::cast_sign_loss)]
        let code_8bit = if let Some(code_32bit) = &self.code {
            (code_32bit & 0xFF) as u8
        } else {
            context.shell.last_exit_status()
        };

        if context.shell.in_function() || context.shell.in_sourced_script() {
            let mut result = ExecutionResult::new(code_8bit);
            result.next_control_flow = ExecutionControlFlow::ReturnFromFunctionOrScript;

            Ok(result)
        } else {
            let _ = writeln!(
                context.stderr(),
                "return: can only be used in a function or sourced script"
            );
            Ok(ExecutionResult::invalid_usage())
        }
    }
}

/// 移除位置参数。
pub(crate) struct ShiftCommand {
    /// 要移动的位置数量, 默认为 1。
    n: Option<i32>,
}

impl builtins::Command for ShiftCommand {
    type Error = crate::engine::Error;

    fn new<I>(args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut args = builtins::BuiltinArgs::new(args);
        let n = match args.next_arg() {
            Some(arg) => Some(
                arg.parse()
                    .map_err(|_| format!("shift: {arg}: numeric argument required"))?,
            ),
            None => None,
        };

        if args.next_arg().is_some() {
            return Err("shift: too many arguments".into());
        }

        Ok(Self { n })
    }

    async fn execute(
        &self,
        context: crate::engine::ExecutionContext<'_>,
    ) -> Result<ExecutionResult, Self::Error> {
        let n = self.n.unwrap_or(1);

        if n < 0 {
            return Ok(ExecutionResult::invalid_usage());
        }

        #[expect(clippy::cast_sign_loss)]
        let n = n as usize;

        let args = context.shell.current_shell_args_mut();

        if n > args.len() {
            return Ok(ExecutionResult::invalid_usage());
        }

        args.drain(0..n);

        Ok(ExecutionResult::success())
    }
}

/// 空操作成功命令。
pub(crate) struct TrueCommand {}

impl builtins::SimpleCommand for TrueCommand {
    fn get_content(
        _name: &str,
        content_type: builtins::ContentType,
        _options: &builtins::ContentOptions,
    ) -> Result<String, crate::engine::Error> {
        match content_type {
            builtins::ContentType::DetailedHelp => Ok("Returns a successful exit status.".into()),
            builtins::ContentType::ShortUsage => Ok("true".into()),
            builtins::ContentType::ShortDescription => Ok("true - success".into()),
        }
    }

    fn execute<I: Iterator<Item = S>, S: AsRef<str>>(
        _context: crate::engine::ExecutionContext<'_>,
        _args: I,
    ) -> Result<ExecutionResult, crate::engine::Error> {
        Ok(ExecutionResult::success())
    }
}
