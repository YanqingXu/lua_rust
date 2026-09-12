# lua_rust - Lua 5.1.5 Rust Interpreter

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lua_rust` 是使用 Rust 实现 Lua 5.1.5 解释器的工程。当前已经不只是脚手架：编译器、寄存器 VM、运行时核心、标准库目录和命令行入口已经打通，可以运行包含函数、闭包、表、循环、多返回、vararg、metatable、coroutine 和大量标准库调用的 Lua 脚本。

本项目仍是兼容性原型，不是完整 Lua 5.1 发行版替代品。C API、官方二进制
chunk、动态 C 模块加载和完整一致性验证仍未完成。真实 GC 和任意字节字符串已实现；
公开宿主访问边界仍须接受行为测试与内存安全工具验证。
兼容结论以官方 Lua 5.1.5 与固定的 `lua_cpp@87c15e6` 双 oracle 为准。

## 在线教学：《从 Lua 解释器开始学习 Rust》

> **教学网站：** [https://yanqingxu.github.io/lua_rust/](https://yanqingxu.github.io/lua_rust/)
>
> **版本主线：** [从 v0.0 最小骨架逐步实现 Lua 5.1 运行时](https://yanqingxu.github.io/lua_rust/versions/index.html)

这不是一套只展示最终源码的 API 文档，而是一条可以亲手执行的解释器实现路线。课程把完整目标拆成 20 个独立 checkpoint：从读取 Lua 文件的最小 CLI 开始，依次加入 Value、Lexer、Parser、表达式语义、Bytecode、寄存器 VM、作用域、控制流、Table、函数、闭包、GC、协程、标准库和命令行工具，最终收口到 Lua 5.1 语言运行时。

这套教学用例的核心价值包括：

| 教学价值 | 课程如何体现 | 学习者得到什么 |
|---|---|---|
| **把大型项目变成可掌握的增量** | v0.0–v1.0 每一版只引入一个核心能力，并明确上一版基础、本版增量和暂不支持的边界。 | 不必一次理解完整 VM，可以沿着稳定的认知台阶逐步建立全局视图。 |
| **每个版本都能独立运行和测试** | `book/code/vXX-name/` 保存完整代码快照、示例脚本和测试，不依赖 Git 历史才能还原。 | 可以随时停在某个版本阅读、修改、调试和做实验，也能直接比较相邻版本的设计变化。 |
| **同时学习 Lua 语义与 Rust 表达方式** | 每章把 Lua 行为、Rust 知识、实现设计和替代方案放在同一上下文中。 | `enum`、模式匹配、所有权、借用、生命周期、错误传播和受控 `unsafe` 不再是孤立语法，而是解决真实运行时问题的工具。 |
| **从教学快照映射到真实工程** | 版本主线优先保证清晰，专题章节再对应 `crates/` 中的成熟实现与 crate 边界。 | 既能看懂最小实现，也能理解它如何演进成编译器、VM、运行时、标准库和 CLI 的工程架构。 |
| **强调设计取舍，而不只给出答案** | 课程记录组件的新增、删除与重构原因，并说明替代方案的复杂度和代价。 | 学到的不只是“代码怎么写”，还包括为什么这样设计、何时应该重构以及如何管理技术债务。 |
| **用可观察结果验证理解** | 每版提供验收命令、预期输出、单元测试和集成测试；网站发布前还会检查全部站内链接与锚点。 | 学习过程有明确反馈，可以用失败的测试定位认知或实现偏差，而不是靠阅读后的主观感觉。 |
| **诚实呈现兼容性边界** | 已实现能力、近似行为、显式 deviation 和 v1.1+ 范围被分别记录，不把缺失的 C API、binary chunk 或 native module 包装成已完成。 | 能建立真实的软件兼容性意识，理解“功能存在”“测试通过”和“语义兼容”之间的差别。 |

如果你是 Rust 初中级学习者，建议直接从 [v0.0 最小解释器骨架](https://yanqingxu.github.io/lua_rust/versions/v00-skeleton.html) 开始；如果已经熟悉编译器或 Lua，可以先浏览[版本能力矩阵](https://yanqingxu.github.io/lua_rust/versions/index.html)，再进入感兴趣的 checkpoint 和专题章节。

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

最近深度审计：2026-09-12，提交基线 `503bf99`；固定 C++ oracle 保持
`87c15e6`。审计之后的工作区修复、验证命令、产物和未完成项统一记录在
[审计整改状态](docs/audits/2026-09-12-remediation.md)。历史报告保留当时结论，
不作为当前工作区已完成的证明。

## 当前进度

这里的 Phase 表示实现分层，不等同于 [`plan.md`](plan.md) 的 M0–M5 里程碑。

| 范围 | 已有能力 | 当前边界 |
|---|---|---|
| 基础设施 | 固定双 oracle、进程 runner、质量门、bytecode/VM trace evidence | 新增 non-official 差异签名硬门与独立官方源码执行 lane；最终工作区结果见整改状态 |
| Runtime / GC | Heap 与 StateArena 统一拥有、真实回收、增量阶段、弱表、finalizer、资源关闭 | 本轮重新打开公开借用与 payload mutation 安全合同；定向默认 Miri 已有证据，不能外推所有 unsafe 安全 |
| 编译器 | 字节 lexer/parser/codegen、38-opcode metadata、递归 Proto 与调试信息 | 既有有效语料 131 项 structured parity；不代表全部输入或官方运行时通过 |
| VM | 调用、闭包、metamethod、coroutine、native continuation、真实 trace | 官方源码执行暴露的 GC/栈窗口、debug hook 与错误来源问题继续逐项收口 |
| 标准库 | 9 个库、字节字符串、真实标准流与文件 IO | 本轮修复参数、浮点、表与数字显示；OS/locale 近似、binary chunk、native module 仍开放 |
| CLI / 工具 | 脚本、REPL、bytecode viewer、差分和结构化产物 | 按具体 case 验收兼容性，不能以整体测试数量代替语义合同 |

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
- Metatable：`__index`、`__newindex`、`__call`、算术、拼接、比较、`__len`、`__tostring` 的主要 dispatch；弱表和 `__gc` 已接入真实 GC；官方执行验收仍在继续。
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

[原始审计](docs/audits/2026-09-12.md) 在 `503bf99` 上验证了完整本地 M0/M1，
并发现测试通过仍未覆盖的公开 API 安全与 Lua 行为问题。审计后新增回归和修复，
当前命令、统计和证据统一见[整改状态](docs/audits/2026-09-12-remediation.md)。

```powershell
powershell -NoProfile -File tools/rust_quality_gate.ps1
powershell -NoProfile -File tools/m0_compatibility_gate.ps1 -CppRoot <固定oracle源码路径>
```

基础 manifest 为 132 项：101 个 non-official、24 个 official、7 个 differential。
non-official 执行 92 个入口、跳过 9 个 helper；固定 case/退出状态/比较通道及历史差异
签名由 `tests/compatibility/non-official-debt.json` 约束。修好一个用例后必须移除其宽限，
新增回归直接失败。

官方 24 文件另由[源码执行 lane](docs/audits/official-source-lane.md) 分类为
19 个执行入口、2 个 helper、3 个显式排除项。库存、编译成功、执行到完成标记是不同
层次的证据。完整 `all.lua`、TestC 和受内存约束的超大负载尚未验收。

---

## 已知限制

| 领域 | 当前限制 | 跟踪 |
|---|---|---|
| Lua 5.1 完整兼容性 | non-official 的具体剩余债务以整改状态为准；官方源码 lane 仍有明确失败、超时和排除项。 | [M0 收口报告](docs/rust_migration/m0_report.md)、M2 总门槛 |
| `_VERSION` | 项目有意返回 `Lua 5.1 (C core prototype)`，不是 stock 的 `Lua 5.1`。 | [NOTE-001](docs/rust_migration/deviation_log.md#note-001-项目扩展-_version-值) |
| GC 与 runtime lifecycle | Runtime shutdown、collector provenance、StateHandle identity/retirement、checked open-Upvalue owner、`PendingState` 临时状态根、coroutine activation、production mutation barrier、weak/finalizer/resurrection、Lua-visible full STW、五阶段 incremental step、managed allocator live/peak/total 与 allocation-triggered automatic GC 已完成；count 继续使用 collector accounted bytes，close 后 allocator live 为 0。main-state owner 与显式 IO drain 已实现；本轮公开访问和 GC 语义修复的证据见整改状态。自动 GC 暂停上界和可恢复 OOM 尚未证明。 | [NOTE-002](docs/rust_migration/deviation_log.md#note-002-gc-增量与-allocator-可观察行为尚未闭环)、[NOTE-009](docs/rust_migration/deviation_log.md#note-009-runtime-与-coroutine-所有权未闭环) |
| 任意字节字符串 | ByteString/GcString/StringPool 与主要编译器/stdlib/IO 边界已支持 invalid UTF-8、NUL 和 high bytes；所有生产字符串强制 canonical interning、binary chunk 与 C API 边界尚未完成。 | [NOTE-007](docs/rust_migration/deviation_log.md#note-007-lua-字符串尚未采用任意字节表示)、[NOTE-010](docs/rust_migration/deviation_log.md#note-010-lua-字符串-intern-hash-选择固定-c-的前向采样) |
| 默认标准流 | 已使用宿主标准流，并有 bytes/process/资源关闭测试；平台行为和后续变更须保持差分验证。 | [NOTE-004](docs/rust_migration/deviation_log.md#note-004-默认标准流从-memory-file-迁移到宿主流) |
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

[CI workflow](.github/workflows/ci.yml) 包含 Windows M0/M1/quality、固定 oracle、
基线变更策略，以及 Linux Miri/ASan。2026-09-01 的提交基线
[远程运行](https://github.com/YanqingXu/lua_rust/actions/runs/33460155464) 三个 job 成功；
这不是后续审计整改已通过远程 CI 的证明。

quality 显式执行 fmt、all-targets Clippy、Debug/Release、doctests、warning-free rustdoc
及 cargo-audit。nextest 不能替代 doctests。Miri 增加公开 Runtime 借用、受管 payload
写入、删除键生命周期以及真实源码和标准库回调路径。

完整本地门与部分 smoke 分开记录；使用跳过开关不能得到完整门通过结论。

---

## 文档

| 文档 | 说明 |
|---|---|
| [在线教程源码](book/src/PREFACE.md) | 《从 Lua 解释器开始学习 Rust》：沿 Value、Table、Lexer、AST、Bytecode 与 VM 阅读真实实现。 |
| [执行计划](plan.md) | 从 M0 验证基线到 M5 发布治理的依赖顺序和验收门。 |
| [审计整改状态](docs/audits/2026-09-12-remediation.md) | 当前工作区修复、最终验收证据和开放风险。 |
| [M0 收口报告](docs/rust_migration/m0_report.md) | 历史收口结果与当时的 fixture/parity、债务和 CI 边界。 |
| [M3 合同设计](docs/rust_migration/m3_contract_design.md) | 两种 binary chunk 格式、验证器、公开 C ABI 和模块生存期的首批设计。 |
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

本地预览教程：

```powershell
cargo install mdbook --version 0.5.4 --locked
mdbook serve book --open
```

推送 `book/` 或被教程引用的 `crates/` 代码后，GitHub Pages workflow 会自动
构建并发布静态站点。

---

## 许可证

MIT License - 详见 [LICENSE](LICENSE)。
