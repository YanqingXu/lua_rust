# lua_rust - Lua 5.1 Rust Migration

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lua_rust` 是 `lua_cpp` 的 Lua 5.1 解释器 Rust 迁移工程。当前不是可用的完整解释器，而是已完成 Phase 0 工程基础设施、并已启动 Phase 1 Runtime Core 的迁移工作区。

最近审计日期：2026-06-14。

---

## 当前进度

| 范围 | 状态 | 说明 |
|---|---|---|
| Phase 0: Project Infrastructure | 已完成 | Rust workspace、6 个 crate 脚手架、CI workflow、质量脚本、迁移文档目录和 fixture 目录已建立。 |
| Phase 1: Runtime Core | ✅ 已完成 | `lua_core` 完整实现 P1.1 类型系统、P1.2 GC 基础设施、P1.3 标记-清除 GC、P1.4 全部核心对象模型 (Table/Metatable/Proto/Function/Upvalue/Userdata/Thread)。 |
| Phase 2: Compiler | 未开始 | `lua_compiler` 仅保留 crate 和模块规划注释，lexer/parser/AST/codegen 未实现。 |
| Phase 3: VM | 未开始 | `lua_vm` 仅保留 crate 和模块规划注释，LuaState、栈、调用帧、opcode dispatch 未实现。 |
| Phase 4: Standard Library | 未开始 | `lua_stdlib` 仅保留 crate 和模块规划注释。 |
| Phase 5: CLI / tools | 未开始 | `lua_app` 与 `lua_bytecode` 是 placeholder binary，不执行 Lua 脚本，也不输出真实字节码。 |

---

## 已实现与未实现

### 已实现的工程基础

- Rust workspace：`lua_core`、`lua_compiler`、`lua_vm`、`lua_stdlib`、`lua_app`、`lua_bytecode` 6 个成员 crate。
- 工具链配置：`rust-toolchain.toml` 使用 stable，workspace `rust-version = "1.96"`，默认目标为 `x86_64-pc-windows-msvc`。
- CI 配置：`.github/workflows/ci.yml` 包含 Windows quality-gate job，步骤为环境检查、fmt、clippy、build、nextest、doc、cargo-audit 和质量脚本。
- Runtime core 基础：Lua 基础类型、`ValueType`/`GcObjectType`/`GcColor`、`Value` enum、Lua truthiness、指针身份相等、Display/toString 风格输出。
- GC 基础设施：`GcRef<T>`、`GcObjectHeader`、`GcObject` unsafe trait、`GarbageCollector`（完整标记-清除循环：mark/propagate/sweep、弱表清理、写屏障、终结器框架）、`GcStrategy`、`MarkSweepGc`/`IncrementalGc` 策略接口。
- 字符串驻留：`GcString`、Lua 5.1 风格字符串哈希、`StringPool::intern/find/remove`。
- 测试基础：`lua_core` 单元测试与 integration test 已覆盖 Value、GC 基础、字符串对象和字符串驻留。

### 尚未实现的 Lua 5.1 语义

- 完整标记-清除 GC：✅ P1.3 已实现 `collect()` 返回回收数量、类型感知 sweep（String/Table）、弱表清理、写屏障和对象释放。终结器框架已就位，待 Userdata 实现后启用。
- 核心对象模型：`Table`（数组/哈希混合存储、元表引用、`#` 长度运算符、`next()` 迭代器、GC 标记）、`Metatable`（17 种 TMS 枚举、flags 缓存查找）、`Proto`（字节码、常量表、调试信息）、`Function`（C/Lua 闭包、上值管理、环境表）、`Upvalue`（Open/Closed 状态）、`Userdata`（GC 管理字节缓冲区、元表、析构器）、`Thread`（协程状态、resume 链）全部已实现。
- 编译器：opcode、instruction 编解码、lexer、parser、AST、bytecode codegen 均未实现。
- VM：LuaState、GlobalState、值栈、CallInfo、38 opcode dispatch、调用/返回、trace/debug 均未实现。
- 标准库：base、math、string、table、io、os、coroutine、debug、package 等模块未实现。
- CLI 与工具：REPL、脚本执行、字节码 dump/JSON 输出未实现。
- 与 C++ 基准的行为对齐：当前仅有结构和部分 runtime core 类型级对齐；bytecode diff 和 VM trace diff 需要 Phase 2/3 之后才能生效。

---

## Crate 说明

| Crate | 类型 | 当前职责与状态 |
|---|---|---|
| `lua_core` | lib | Phase 1 目标 crate。已实现全部核心对象模型：基础类型/Value、GC 基础设施、GcString/StringPool、Table、Metatable、Proto、Upvalue、Function、Userdata、Thread。 |
| `lua_compiler` | lib | Phase 2 目标 crate。目前只有脚手架和 C++ -> Rust 模块映射注释；未导出 opcode、lexer、parser、AST 或 codegen 模块。 |
| `lua_vm` | lib | Phase 3 目标 crate。目前只有脚手架和模块规划；未实现 LuaState、执行循环、opcode handlers、调用帧或 trace。 |
| `lua_stdlib` | lib | Phase 4 目标 crate。目前只有脚手架和标准库模块规划；未实现任何 Lua 标准库。 |
| `lua_app` | bin | Phase 5 目标 binary。目前只打印 placeholder 信息；不支持 REPL 或脚本执行。 |
| `lua_bytecode` | bin | Phase 5 目标 binary。目前只打印 placeholder 信息；不编译 Lua 源码，也不输出真实字节码。 |

### 依赖关系现状

内部 crate 依赖目前大多尚未启用，`Cargo.toml` 中以 phase-gated 注释保留。`lua_core` 是当前唯一承载实际迁移实现的 crate。

---

## 快速开始

### 环境要求

- Rust stable，workspace 要求 `rust-version = "1.96"`。
- Windows x64 MSVC 是当前主开发/CI 平台。
- 完整 CI 门禁还需要 `cargo-nextest` 和 `cargo-audit`。

### 本地基础验证

这些命令不依赖 C++ 基准构建产物，也不依赖 `cargo-audit`：

```powershell
cd lua_rust

cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
cargo doc --no-deps
```

当前审计结果：上述命令均通过；`cargo test --workspace` 共运行 190 个 Rust 测试，测试集中在 `lua_core`。

### CI 对齐门禁

CI workflow 和 `tools/rust_quality_gate.ps1` 的目标门禁包含：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_env_check.ps1
cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo nextest run --workspace
cargo doc --no-deps
cargo audit
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1
```

本地可按阶段跳过暂不适用的步骤：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1 -SkipAudit -SkipCrossValidate
```

注意：当前脚本实际调用 `cargo nextest run --workspace`。如果本机未安装 `cargo-nextest`，或遇到 `.cargo/config.toml` 中 `nextest` alias 导致的递归错误，可先使用 `cargo test --workspace` 作为本地回退验证。

### Placeholder binaries

当前 binary 只用于确认脚手架可构建：

```powershell
cargo run -p lua_app
cargo run -p lua_bytecode
```

它们不会执行 Lua 脚本，也不会生成字节码。

---

## CI 与 C++ 基准对齐

`.github/workflows/ci.yml` 当前包含两个 job：

| Job | 状态 | 说明 |
|---|---|---|
| `quality-gate` | 已配置 | Windows 上运行 Rust 工具链安装、环境检查、fmt、clippy、build、nextest、doc、audit 和质量脚本。 |
| `cross-validate` | 已配置但禁用 | `if: false`，计划在 Phase 2+ 且 CI 中可用 C++ 基准产物后启用。 |

`tools/compare_bytecode.ps1` 和 `tools/compare_vm_trace.ps1` 已存在，但当前对齐状态为 N/A：

- `lua_cpp` 源码目录存在于 `..\lua_cpp`。
- `..\lua_cpp\bin` 当前不存在，因此 C++ `lua_bytecode.exe` / `lua_app.exe` 不可用。
- Rust 侧 compiler、VM、CLI 仍未实现，因此 bytecode diff 和 VM trace diff 还不能代表 Lua 语义兼容性。
- `docs/rust_migration/deviation_log.md` 当前没有登记任何已批准行为偏差。

---

## 项目结构

```text
lua_rust/
├── Cargo.toml
├── rust-toolchain.toml
├── .cargo/config.toml
├── .github/workflows/ci.yml
├── crates/
│   ├── lua_core/
│   │   ├── src/types.rs
│   │   ├── src/value.rs
│   │   ├── src/table.rs
│   │   ├── src/metatable.rs
│   │   ├── src/gc/
│   │   ├── src/gc_string.rs
│   │   └── src/string_pool.rs
│   ├── lua_compiler/
│   ├── lua_vm/
│   ├── lua_stdlib/
│   ├── lua_app/
│   └── lua_bytecode/
├── tests/fixtures/
│   ├── phase_1/
│   ├── phase_2/
│   ├── phase_3/
│   ├── phase_4/
│   └── phase_5/
├── tools/
│   ├── rust_env_check.ps1
│   ├── rust_quality_gate.ps1
│   ├── compare_bytecode.ps1
│   └── compare_vm_trace.ps1
└── docs/
    ├── glossary.md
    └── rust_migration/
```

---

## 开发约定

- 类型使用 `CamelCase`，函数/方法/模块使用 `snake_case`，常量使用 `UPPER_CASE`。
- `unsafe` 仅用于 GC、裸指针/FFI 等边界；workspace lint 开启 `unsafe_op_in_unsafe_fn = "deny"` 和 `clippy::undocumented_unsafe_blocks = "deny"`。
- 迁移应优先保持与 `lua_cpp` 行为一致；所有批准的行为差异必须登记到偏差日志。
- Phase 推进时先更新对应 crate 的真实实现，再更新 README 和迁移文档，避免文档先行宣称未落地功能。

---

## 文档

| 文档 | 说明 |
|---|---|
| [Phase 1 进度报告](docs/rust_migration/phase_1_progress.md) | Phase 1 Runtime Core 整体进度跟踪。 |
| [P1.4 Table 任务](docs/rust_migration/tasks/P1.4-table.md) | Table 核心结构与基础操作迁移说明。 |
| [P1.4 Metatable 任务](docs/rust_migration/tasks/P1.4-metatable.md) | Metatable/TMS 系统迁移说明。 |
| [偏差日志](docs/rust_migration/deviation_log.md) | Rust 与 C++ 基准之间已批准行为偏差的登记表；当前为空。 |
| [术语表](docs/glossary.md) | Lua 概念与项目术语说明。 |

---

## 许可证

MIT License - 详见 [LICENSE](LICENSE)。
