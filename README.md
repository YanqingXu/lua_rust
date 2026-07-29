# lua_rust - Lua 5.1.5 Rust Interpreter

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lua_rust` 是使用 Rust 实现 Lua 5.1.5 解释器的工程。当前已经不只是脚手架：编译器、寄存器 VM、运行时核心、标准库目录和命令行入口已经打通，可以运行包含函数、闭包、表、循环、多返回、vararg、metatable、coroutine 和大量标准库调用的 Lua 脚本。

本项目仍是兼容性原型，不是完整 Lua 5.1 发行版替代品。C API、官方二进制
chunk、动态 C 模块加载、真实 GC 闭环、任意字节字符串和完整一致性测试仍未完成。
兼容结论以官方 Lua 5.1.5 与固定的 `lua_cpp@87c15e6` 双 oracle 为准。

## 项目目标

本项目的目标是使用 Rust 实现 Lua 5.1.5 版本的解释器。实现过程优先追求代码可读性、架构设计简洁性，以及可作为 Rust 学习材料和工程实践参考的价值。

换句话说，`lua_rust` 不只关注“能跑 Lua”，也关注解释器各个组成部分是否容易阅读、调试、扩展和讲解：从词法/语法分析、字节码生成、寄存器 VM、调用栈、闭包/upvalue、GC，到标准库和命令行入口，都尽量保留清晰的模块边界和贴近 Rust 习惯的实现方式。

## Rust 教学路线

建议把本项目当作一条从 Rust 基础到解释器工程的阅读路线，而不是一次性读完整个仓库。

| 阶段 | 建议阅读入口 | 主要学习点 |
|---|---|---|
| 1. 值与类型系统 | `crates/lua_core/src/value.rs`、`types.rs` | 用 `enum`、模式匹配和显式类型建模动态语言的运行时值。 |
| 2. 共享运行时对象 | `string_pool.rs`、`table.rs`、`function.rs`、`proto.rs` | 理解所有权、共享引用、可变状态和解释器对象之间的边界。 |
| 3. GC 与生命周期取舍 | `crates/lua_core/src/gc/` | 观察 Rust 静态生命周期与动态语言 GC 模型之间的工程折中。 |
| 4. 词法与语法分析 | `crates/lua_compiler/src/lexer.rs`、`parser/`、`ast/` | 用结构化数据表达源码、Token、AST 和语法错误。 |
| 5. 字节码生成 | `crates/lua_compiler/src/codegen/`、`opcode.rs` | 学习寄存器分配、作用域管理、跳转回填和 Lua 5.1 opcode 编码。 |
| 6. 虚拟机执行 | `crates/lua_vm/src/execute.rs`、`state/` | 通过 opcode dispatch 理解栈、调用帧、闭包、upvalue、多返回和 coroutine。 |
| 7. Rust 宿主函数接口 | `crates/lua_stdlib/src/` | 学习如何把 Rust 函数包装成 Lua 可调用的标准库能力。 |
| 8. 命令行与工具化 | `crates/lua_app/src/main.rs`、`crates/lua_bytecode/src/main.rs` | 了解库代码如何变成可运行工具、REPL 和调试字节码输出。 |

如果只是学习 Rust，可以先读第 1、2、4 阶段；如果关注解释器实现，建议按 1 到 8 顺序阅读；如果关注工程实践，可以重点观察 crate 分层、测试布局、错误传播和文档注释如何配合。

最近状态审计日期：2026-07-26；Rust 基线为 `6284135`，C++ oracle 为
`87c15e6`。

---

## 当前进度

这里的 Phase 是实现分层，不等同于 [`plan.md`](plan.md) 的 M0–M5 里程碑。
状态由实现证据、项目内测试和 oracle 验证共同决定；“模块存在”不会自动记为完成。

| 范围 | 状态 | 证据驱动结论 |
|---|---|---|
| Phase 0: Project Infrastructure | ✅ M0 本地完成 | 固定双 oracle、统一质量/兼容门、当前 131-file manifest、进程 runner 与 parity 工具均已建立；M0 收口时完整 gate 通过，新增 M1 fixtures 的外部 cwd Smoke 复验通过，远程 CI 尚待首次运行。 |
| [Phase 1: Runtime Core](docs/rust_migration/phase_1_report.md) | 🟡 partial | ByteString、GcRef ObjectId provenance、managed Proto roots、checked open-Upvalue owner、确定性 shutdown、临时对象/状态根以及 compiler、library/package、IO publication 已落地；VM/app/results publication 与真实 sweep/barrier/finalizer 仍开放。 |
| [Phase 2: Compiler](docs/rust_migration/phase_2_report.md) | 🟡 partial | Byte lexer/parser/codegen、38-opcode metadata 与简单 bytecode 形状已对齐；nested Proto、结构化极限错误和逐 opcode parity 未完成。 |
| [Phase 3: VM](docs/rust_migration/phase_3_report.md) | 🟡 partial | 主要 dispatch、调用、闭包、metamethod、coroutine、managed active Proto、checked open-Upvalue owner 与 Runtime coroutine trampoline 路径存在；debug/protected-helper 跨 state、全量 trace parity 和 host ABI 未完成。 |
| [Phase 4: Standard Library](docs/rust_migration/phase_4_report.md) | 🟡 partial | 9 个库入口和大量函数有项目内测试；GC/dump/OS/string/native module 等已登记差异仍开放。 |
| [Phase 5: CLI / tools](docs/rust_migration/phase_5_report.md) | 🟡 partial | M0 进程 runner、双 oracle、bytecode JSON schema v2 和 parity 工具已落地；88 个非官方差异、C++ local-name 证据、nested bytecode/真实 VM trace parity 和成熟 REPL 仍未完成。 |

---

## 已实现并有项目内测试的能力

以下列表说明已有实现覆盖面，不是 Lua 5.1 完整兼容声明。各项的完成判定和
阻塞证据见 Phase 报告与
[兼容偏差日志](docs/rust_migration/deviation_log.md)。

### 语言与 VM

- 基础表达式：nil/boolean/number/string、算术、比较、逻辑、取长、字符串拼接。
- 变量与作用域：global/local/block local、赋值、多重赋值、索引赋值、字段赋值。
- 控制流：`if/elseif/else`、`while`、`repeat until`、`break`、`do`、numeric for、generic for。
- 函数系统：函数声明、局部函数、函数表达式、方法定义/调用、递归、tail call。
- 闭包与 upvalue：嵌套闭包、共享 upvalue、关闭 open upvalue。
- 调用语义：Lua 函数、C 函数、多返回、最终调用展开、非最终多返回折叠、vararg、Lua 5.1 风格 `arg` 表。
- 表：数组/哈希/混合构造器、大数组 `SETLIST`、成员访问、索引访问、`next/pairs/ipairs`。
- Metatable：`__index`、`__newindex`、`__call`、算术、拼接、比较、`__len`、`__tostring` 的主要 dispatch；弱表和 `__gc` 只有组件/项目内路径，真实 GC 语义尚未验收。
- Coroutine：`create/resume/yield/status/running/wrap` 的基本流程可用。

### 标准库注册面

下表表示函数已经注册或有实现入口；它不保证所有参数、错误、平台行为和 byte
语义均与双 oracle 一致。

| 模块 | 已注册能力 |
|---|---|
| base | `assert`、`collectgarbage`、`dofile`、`error`、`gcinfo`、`getfenv`、`getmetatable`、`ipairs`、`load`、`loadfile`、`loadstring`、`newproxy`、`next`、`pairs`、`pcall`、`print`、`rawequal`、`rawget`、`rawset`、`select`、`setfenv`、`setmetatable`、`tonumber`、`tostring`、`type`、`unpack`、`xpcall` |
| math | `abs`、`acos`、`asin`、`atan`、`atan2`、`ceil`、`cos`、`cosh`、`deg`、`exp`、`floor`、`fmod/mod`、`frexp`、`ldexp`、`log`、`log10`、`max`、`min`、`modf`、`pow`、`rad`、`random`、`randomseed`、`sin`、`sinh`、`sqrt`、`tan`、`tanh`、`huge`、`pi` |
| string | `byte`、`char`、`dump`、`find`、`format`、`gmatch/gfind`、`gsub`、`len`、`lower`、`match`、`rep`、`reverse`、`sub`、`upper`；包含一套 Lua pattern 近似实现。 |
| table | `concat`、`foreach`、`foreachi`、`getn`、`insert`、`maxn`、`remove`、`sort` |
| io | `tmpfile`、`open`、`input`、`output`、`read`、`write`、`lines`、`flush`、`close`、`type`，以及文件句柄的 `read/write/seek/close/setvbuf/lines/flush`。 |
| os | `clock`、`date`、`difftime`、`execute`、`remove`、`rename`、`setlocale`、`time`、`tmpname` |
| coroutine | `create`、`resume`、`running`、`status`、`wrap`、`yield` |
| debug | `getinfo`、`getupvalue`、`setupvalue`、`getlocal`、`setlocal`、`gethook`、`sethook`、`traceback`、`getregistry`、`getfenv`、`setfenv`、`setmetatable` |
| package | `require`、`module`、`package.loaded`、`package.preload`、`package.path`、`package.loadlib` 占位错误返回、`package.seeall` |

---

## 测试状态

2026-07-26 当前 M0 工作树在本地统一质量入口执行：

```powershell
powershell -File tools\rust_quality_gate.ps1 -SkipAudit
```

结果为 fmt、all-targets Clippy、Debug/Release test 和 `-D warnings` rustdoc
全部通过；Debug 与 Release 各验证 596 个 workspace tests。`-SkipAudit`
表示这次本地证据没有执行依赖漏洞审计；远程 CI 尚待首次运行。这些项目内测试
不代表官方 Lua 5.1 或 C++ oracle 已完整一致。

| Crate | 单元测试 | 集成测试 | 当前总计 |
|---|---:|---:|---:|
| `lua_core` | 258 | 41 | 299 |
| `lua_compiler` | 115 | 66 | 181 |
| `lua_vm` | 6 | 29 | 35 |
| `lua_stdlib` | 0 | 78 | 78 |
| `lua_app` | 0 | 3 | 3 |
| `lua_bytecode` | 0 | 0 | 0 |
| **总计** | **379** | **217** | **596** |

历史提交基线还记录了以下命令行 smoke test：

```powershell
cargo run -q -p lua_app -- examples\more_tests.lua
cargo run -q -p lua_app -- -e "local t={3,1,2}; table.sort(t); print(table.concat(t, ','))"
cargo run -q -p lua_app -- -e "print(string.gsub('a1 b2','(%a)(%d)','%2%1'))"
cargo run -q -p lua_bytecode -- examples\more_tests.lua
```

它们在当时可运行；`lua_bytecode` 对 `examples/more_tests.lua` 输出 27 条指令、
14 个常量。Smoke test 与 Rust 单元测试都不能替代 fixture manifest、进程级
stdout/stderr/exit/timeout 断言、official suite 或双 oracle differential。

当前 manifest 共登记 131 个 fixture：101 个 non-official、24 个 official 和
6 个 differential（4 个 M0 focused cases，加 2 个 M1 raw-byte cases）。101 个
non-official 中执行 92 个、按分类跳过 9 个 helper；
92/92 退出码一致，raw 全通道匹配 4/92，差异 88，runner error 和 timeout 均为
0。仅用于分析的显式路径加 EOL 规范化后，全通道一致为 75/92；runner 没有用
这项分析指标掩盖 raw 差异。

4 个 focused case 在官方 Lua 5.1.5 与 `lua_cpp@87c15e6` 两条 lane 上均为
4/4；官方 `_VERSION` 差异由
[NOTE-001](docs/rust_migration/deviation_log.md#note-001-项目扩展-_version-值)
显式批准。本地统一 M0 gate 用时 11,765 ms，`passed=true`、
`hardFailures=0`，并保留 3 项开发债务。完整证据与边界见
[M0 收口报告](docs/rust_migration/m0_report.md)。

---

## 已知限制

| 领域 | 当前限制 | 跟踪 |
|---|---|---|
| Lua 5.1 完整兼容性 | M0 验证闭环已建立，但 non-official 仍有 88 个语义差异，official strict/slow suite 也尚未全通过。 | [M0 收口报告](docs/rust_migration/m0_report.md)、M2 总门槛 |
| `_VERSION` | 项目有意返回 `Lua 5.1 (C core prototype)`，不是 stock 的 `Lua 5.1`。 | [NOTE-001](docs/rust_migration/deviation_log.md#note-001-项目扩展-_version-值) |
| GC 与 runtime lifecycle | Runtime shutdown、collector provenance、StateHandle identity/retirement、checked open-Upvalue owner、`PendingState` 临时状态根与 coroutine activation trampoline 切片已完成，但 stdlib collect path 仍不执行真实 sweep，计数/step 为模拟；main-state owner、其余生产 publication、debug/protected-helper 跨 state、barrier/finalizer 未闭环。 | [NOTE-002](docs/rust_migration/deviation_log.md#note-002-gc-可观察行为尚未形成真实回收闭环)、[NOTE-009](docs/rust_migration/deviation_log.md#note-009-runtime-与-coroutine-所有权未闭环) |
| 任意字节字符串 | ByteString/GcString/StringPool 与主要编译器/stdlib/IO 边界已支持 invalid UTF-8、NUL 和 high bytes；所有生产字符串强制 canonical interning、binary chunk 与 C API 边界尚未完成。 | [NOTE-007](docs/rust_migration/deviation_log.md#note-007-lua-字符串尚未采用任意字节表示)、[NOTE-010](docs/rust_migration/deviation_log.md#note-010-lua-字符串-intern-hash-选择固定-c-的前向采样) |
| 默认标准流 | 提交基线使用 memory file；宿主标准流修复已通过本地 process tests、完整 M0 fixture gate 与 4-case 双 oracle，尚待远程 CI 首次运行。 | [NOTE-004](docs/rust_migration/deviation_log.md#note-004-默认标准流从-memory-file-迁移到宿主流) |
| C API / FFI | 没有 `lua_capi` crate、公开 headers、稳定 ABI 或 C consumer。 | [NOTE-008](docs/rust_migration/deviation_log.md#note-008-lua-51-c-apiabi-尚不存在) |
| 二进制 chunk | 旧进程内伪 dump registry 已删除；`string.dump` 在真实 Lua 5.1 serializer 完成前显式返回 unsupported。 | [NOTE-003](docs/rust_migration/deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk) |
| 动态模块 | `package.loadlib` 明确返回 unsupported。 | [NOTE-005](docs/rust_migration/deviation_log.md#note-005-packageloadlib-明确不支持动态库) |
| OS/locale/time | clock、时区/DST、locale、格式符和错误 tuple 是有限近似。 | [NOTE-006](docs/rust_migration/deviation_log.md#note-006-oslocale-与-time-使用平台无关近似) |
| 编译器错误恢复 | 某些 malformed/extreme codegen 输入仍可能触发内部 panic。 | M2.1 |
| CLI/工具 | 原两个 bytecode case 的 opcode/constant/metadata 已一致，但固定 C++ printer 缺 local names，nested closure 仍有大量真实差异；真实 VM trace 因两端缺少 `--trace-diff` 支持而保持债务。 | M2.2、M2.7、M2.16–M2.18 |

---

## 快速开始

```powershell
cd lua_rust

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace
cargo test --workspace --release
$env:RUSTDOCFLAGS = "-D warnings"
cargo doc --workspace --no-deps
Remove-Item Env:RUSTDOCFLAGS

# 运行 Lua 脚本
cargo run -p lua_app -- examples\more_tests.lua

# 执行一段源码
cargo run -p lua_app -- -e "print(math.sqrt(81))"

# 进入 REPL
cargo run -p lua_app

# 字节码 dump
cargo run -p lua_bytecode -- examples\more_tests.lua
cargo run -p lua_bytecode -- examples\more_tests.lua --format=json
```

---

## Crate 说明

| Crate | 类型 | 职责 | 2026-07-26 本地测试 |
|---|---|---|---:|
| `lua_core` | lib | 类型系统、GC、字符串池、Table/Metatable/Proto/Function/Upvalue/Userdata/Thread。 | 299 |
| `lua_compiler` | lib | Opcode、Lexer/Token、AST、Parser、CodeGen。 | 181 |
| `lua_vm` | lib | LuaState、Stack、CallInfo、opcode dispatch、调用/返回/协程/元方法执行。 | 35 |
| `lua_stdlib` | lib | base/math/string/table/io/os/coroutine/debug/package 标准库实现。 | 78 |
| `lua_app` | bin | Lua 5.1 命令行 runner、stdin、`-e/-l/-i`、脚本参数、REPL。 | 3 |
| `lua_bytecode` | bin | Lua 源码到 `Proto` 的 text/JSON 字节码查看器。 | 0 |

---

## CI 与质量门

`.github/workflows/ci.yml` 已接入固定 C++ oracle、M0 compatibility gate、
oracle baseline-change policy、Rust quality gate 和结构化 artifact 上传。
本地已验证统一入口；远程 workflow 尚待首次运行。

| Job | 当前证据 | 内容 |
|---|---|---|
| `M0 compatibility gate` | M0 收口完整运行：11,765 ms、0 hard failure、3 debts；当前 131-file manifest 的外部 cwd Smoke 复验同为 0 hard failure、3 debts | 固定 oracle、runner 自检、完整 fixture inventory、4×2 focused differential、101 non-official 和 parity artifact。 |
| `quality-gate` | 本地 `-SkipAudit` 通过；远程待首次运行 | fmt、all-target clippy、build、596 个 Debug/Release tests、doc 和 audit 分类。 |

本地通过不等于远程 CI 已验证；首次 CI run 仍需确认依赖审计、checkout、
artifact 上传和 workflow 环境本身。

---

## 文档

| 文档 | 说明 |
|---|---|
| [执行计划](plan.md) | 从 M0 验证基线到 M5 发布治理的依赖顺序和验收门。 |
| [M0 收口报告](docs/rust_migration/m0_report.md) | 本地门禁结果、fixture/differential/parity 证据、3 项债务与 CI 边界。 |
| [ByteString RFC](docs/rust_migration/byte_string_rfc.md) | M1 字节表示、pointer+length、hash 与迁移边界决策。 |
| [Phase 0 报告](docs/rust_migration/phase_0_report.md) | 基础设施初始化的历史报告；不代表当前 M0 质量门已经通过。 |
| [Phase 1 报告](docs/rust_migration/phase_1_report.md) | Runtime Core 的实现证据、缺口与阻塞。 |
| [Phase 2 报告](docs/rust_migration/phase_2_report.md) | Compiler 的实现证据、缺口与阻塞。 |
| [Phase 3 报告](docs/rust_migration/phase_3_report.md) | VM 的实现证据、缺口与阻塞。 |
| [Phase 4 报告](docs/rust_migration/phase_4_report.md) | Standard Library 的实现证据、缺口与阻塞。 |
| [Phase 5 报告](docs/rust_migration/phase_5_report.md) | CLI/tools 的实现证据、缺口与阻塞。 |
| [兼容偏差日志](docs/rust_migration/deviation_log.md) | 固定 `NOTE-*`、oracle、测试/任务和处置状态。 |
| [类型速查表](docs/rust_migration/type_mapping_table.md) | Rust 内部类型、模块和职责速查表。 |
| [术语表](docs/glossary.md) | Lua 概念与项目术语说明。 |

---

## 许可证

MIT License - 详见 [LICENSE](LICENSE)。
