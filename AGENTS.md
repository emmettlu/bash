# bash AI 代理开发指南

本指南帮助 AI 代理高效地在 `bash` 代码库上工作, 提供架构, 模式和开发工作流的基本上下文. 

## 1. 架构概述 & 导航

### 项目结构

本项目是单 crate Rust 项目 (package 名为 "bash", Rust 2024 edition):

- `src/` 主源码目录:
  - `shell/` : CLI 参数, 入口逻辑, 配置加载
  - `engine/` : 核心 Shell 实现, 解释器, 变量, 作业控制, 系统集成
  - `parser/` : Shell 脚本解析 (tokenizer + PEG 生成 AST)
  - `builtins/` : 内置命令实现 (echo, cd, set 等)
  - `interactive/` : 交互输入后端, 补全, 提示符, Windows 终端处理

### 关键文件 & 入口点

**必须先理解的关键文件:**

- `src/main.rs` - 程序入口点, 创建 compio runtime 并调用 `bash::run()`
- `src/lib.rs` - 导出 `run()` 和 `ExitCode`
- `src/shell/entry.rs` - Shell 实例化, 解析参数, 运行交互/脚本模式
- `src/engine/shell.rs` - 核心 `Shell<SE>` 结构体与内部状态
- `src/engine/interp.rs` - AST 执行引擎 (impl Execute for Program/Pipeline 等)
- `src/parser/parse_impl/peg.rs` - PEG 语法规则, token 驱动解析器
- `src/engine/sys/` - Windows 平台集成 (大部分 fallback 到 unsupported)

**架构模式:**

- 使用 `Shell::builder()` 创建 shell 实例, `CreateOptions` 仅作为 crate 内部 builder backing 类型
- 使用 builder 模式集中配置 shell 初始化参数
- 解析基于 peg crate (token 流驱动)
- 执行全程异步 (compio runtime)
- 错误格式化通过对象安全的 `ErrorFormatter` 注入, 避免泛型扩展污染执行路径
- Windows 平台代码集中放在 `engine::sys`

### 模块依赖关系

```text
main.rs
  → lib::run()
    → shell::entry (参数处理 + instantiate_shell)
      → engine::Shell::builder()
      → interactive::{Basic,Minimal}InputBackend
      → parser (脚本解析)
      → engine::interp (AST 执行)
      → engine::builtins (内置命令)
```

## 2. 测试与验证策略

### 推荐开发流程 (内循环)

1. 修改后立即 `cargo check`
2. 运行受影响模块的单元测试: `cargo test`
3. 快速检查格式与 lint:
   ```bash
   cargo fmt -- --check
   cargo clippy
   ```

### 提交前验证 (外循环)

推荐执行:

```bash
cargo fmt
cargo clippy
cargo test
```

### 测试组织

- 单元测试写在各源文件内的 `#[cfg(test)] mod tests { ... }`
- 解析器快照测试位于 `src/parser/snapshot_tests.rs`
- 内置命令和引擎均有大量单元测试
- 目前无 workspace 或 xtask 工具, 直接使用 cargo 命令

**测试驱动建议:**

- 改动前先添加/更新测试描述期望行为
- 先针对单个 crate/module 跑测试, 再扩展范围
- 兼容性行为改动需对应补充测试

**快速迭代技巧:**

- `cargo test <test_name>` 运行单个测试
- `cargo test --lib` 仅库测试
- `cargo check` 最快语法/类型检查

### 测试失败处理

- 优先关注改动区域的失败
- 解析/执行行为变化通常需同步更新快照或测试断言
- 格式与 clippy 问题必须先修复

## 3. 错误处理 & 日志模式

### 错误处理

- crate 内错误使用 `thiserror`
- 测试代码可用 `anyhow`
- 常见错误类型: `engine::Error`, `engine::ErrorKind`, `parser::ParseError`

### 日志与调试输出

使用 `log` + `nanologger` 进行调试日志输出.

预定义分类在 `trace_categories.rs`:

- `COMMANDS`, `EXPANSION`, `FUNCTIONS`, `JOBS`, `PARSE`, `PATTERN`, `UNIMPLEMENTED` 等

用法示例:

```rust
log::debug!(target: trace_categories::JOBS, "polling job {}", job_id);
```

## 4. Windows 平台特殊考虑

- 本项目主要为 Windows 构建 (windows-sys 依赖)
- 大部分进程/信号/管道功能目前 fallback 到 `src/engine/sys/unsupported`
- 文件系统有部分原生实现 (`sys/fs/native.rs`)
- 环境变量处理: USERPROFILE → HOME, TEMP/TMP → TMPDIR 等
- 路径统一使用 `/` 分隔符 (有 `normalize_path_separators` 工具)
- `/dev/null` 等特殊文件映射到 Windows NUL
- 交互终端: `interactive/win_term.rs`
- 许多作业控制, trap, signal 功能当前受限

改动平台相关代码时, 优先在 `engine/sys` 下实现, 避免污染通用路径.

## 5. 文档与示例标准

### rustdoc 要求

- 所有导出的类型, 函数, trait, 模块必须有良好 rustdoc
- 内部组件的文档为尽力而为

### 新功能示例

仅主要特性添加 runnable 示例. 示例应:

- 可通过 `cargo run` 执行
- 包含完整错误处理
- 演示基本与进阶用法

## 6. 性能与克隆策略

- 默认避免克隆
- 仅在异步安全或必须持有独立副本时才克隆

## 快速参考清单

### 开始改动前

- [ ] 理解改动涉及的模块
- [ ] 确认是否会影响公开 API 或 Windows 行为
- [ ] 找到相关测试文件

### 开发中

- [ ] 频繁运行 `cargo check`
- [ ] 先跑受影响模块的测试
- [ ] 必要时更新快照测试

### 提交前

- [ ] `cargo fmt`
- [ ] `cargo clippy`
- [ ] `cargo test` (至少受影响区域)
- [ ] 提交信息纯英文, Conventional Commits 风格, 单段不超过 64 词

### 文档

- [ ] 为导出的公开 API 添加 rustdoc
- [ ] 主要功能提供可运行示例
- [ ] 必要时更新本指南

### 个人规范

- 所有代码注释, 文档, 说明使用简体中文
- 标点符号使用半角 (英文标点), 逗号后追加空格
- 编写完成后仅需运行 `cargo clippy`, 不必 release 构建或手动运行
