---
status: partial
phase: 2
phase_name: Compiler
last_updated: 2026-08-12
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 2 Report: Compiler

## 结论

**状态：partial。** M2.1 compiler error/limit 已完成本地切片：用户可达 codegen panic
已转为结构化错误，parser/error/long-jump/register/constant 极限探针与固定 C++ 对齐。
当前仓库 131 个有效 Lua 源码也已完整 structured 零差异；真实 VM Snapshot/Basic 与
Diff/Basic 已在同一 12-case 代表集达到零差异，`stackTop`/`changedRegisters` 进入完整 M0
硬门；Complete 核心生命周期四例在 Snapshot/Diff 两种模式也均为 4/4 零差异，并观察全部
八类事件。但 M2.2 的逐 opcode 三重证据、M2.3
的全域 Proto/Complete trace 合同与逐 opcode 三重证据仍未全部关闭，因此 Phase 2 不升级为完成。

| 分类 | 本阶段内容 |
|---|---|
| `completed-local` | M2.1 codegen 结构化错误与 compiler limit/error matrix；131-case full-valid structured corpus；M2.7 真实 VM Snapshot/Basic、Diff/Basic 双 12-case parity、Complete 核心双 4-case、callback 双 2-case 与 GC retirement 双 producer 自检；M2.16 CLI 双实现 18/18；M2.17 REPL 双实现 8/8；M2.18 bytecode viewer 15/15。 |
| `partial` | Lexer/token、Parser/AST、38-opcode metadata、CodeGen 与 Proto/trace 全域合同。 |
| `not-started` | 逐 opcode 的 unit + Lua behavior + bytecode/trace 三重矩阵。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Lexer/token | byte-oriented lexer/token 与 byte source API | lexer/byte-source tests、131-case structured corpus | 当前语料与极限矩阵通过；全域 fuzz/budget 由 M4 跟踪 |
| Parser/AST | `parser/`、`ast/` | parser tests + `parser_limit_tests.rs` | M2.1 limits completed-local：递归、block、concat/call、locals/upvalues、malformed method 均锁定 |
| Opcode encoding/metadata | `opcode.rs` 的 38 个枚举项及字段 helper | opcode 单元测试；131-case 覆盖固定 C++ 36/36 frontend-produced opcode | partial：`NOT/TESTSET` 有 negative contract，但逐项三重矩阵仍属 M2.2 |
| CodeGen | `codegen/` 的 scope、register、jump、expression/statement emit 与 `pending_error` | `codegen_error_tests.rs`、publication rollback、raw CLI matrix | M2.1 completed-local：无显式 `panic!`，partial Proto 不发布 |
| Proto output | constants、nested proto、line/local/upvalue/source byte envelope | fresh 131/131 structured parity；300-register 1/1 parity；262145-constant probe；真实 Snapshot/Basic、Diff/Basic 均为 12/12；Complete 核心 Snapshot/Diff 均为 4/4；callback Snapshot/Diff 均为 2/2；双端 retirement 3-open/2-close 自检 | 当前核心 corpus 与 M2.7 lifecycle completed-local；更广 M2.3 仍 partial |

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| 38 opcode 没有逐项 unit + Lua behavior + bytecode/trace 三重证据 | 131-case 覆盖全部 36 个 frontend-produced opcode，但不是逐项验收表 | M2.2 |
| Callback trace 投影 | metamethod/debug callback 的事件顺序、RETURN/caller `stackTop`、CALL depth 与 caller-upvalue changed-register 均已对齐；Snapshot/Diff 双 2-case 零差异并进入 runner | completed-local |
| GC trace-state retirement | Rust 使用带 generation 的 `StateHandle` 并在 arena retirement 交付 close；固定 C++ adapter 通过配置 allocator 捕获 `LuaState` 析构后的精确 deallocation，在安全 trace 边界发布 close 并清 cache。双端 producer 各自锁定 3 open/2 GC close | completed-local |
| 固定 C++ 的 register/constant 字段截断 | 300-register bytecode 零差异；262145-constant 双端均接受，未产生错误 | oracle 行为已锁定；production compiler budget 由 M4.2 跟踪 |
| bytecode viewer | compact/full/CFG/diff 的 12 个跨语言 case 与 JSON/strict-format 3 个扩展 case 全绿；高位 constant text/diff 保持 raw byte | M2.18 completed-local |

## Oracle 与验收

- 语法和 chunk 语义比较官方 Lua 5.1.5；项目扩展和精确 bytecode/Proto 结构比较
  `lua_cpp@87c15e6`。
- M2.1 fresh 证据为 `target/compatibility/m2-compiler-limit-probes/`、
  `target/compatibility/m2-compiler-limits-bytecode-131-final/report.json` 和
  `target/compatibility/m2-compiler-limits-final-m0-pinned/gate.json`；后者为 0 hard failure。
- M2.7 Snapshot/Basic fresh 证据为
  `target/compatibility/m2-real-trace-observational-final-m0-pinned/vm-trace-snapshot-basic/report.json`，
  12/12 passed、0 difference、0 infrastructure failure。
- M2.7 Diff/Basic fresh 证据为
  `target/compatibility/m2-real-trace-lifecycle-pinned/vm-trace-diff-basic/report.json`，
  12/12 passed、0 difference、0 infrastructure failure；同次 M0 也重验 Snapshot 12/12。
- M2.7 Complete/Diff fresh 证据为
  `target/compatibility/m2-io-final-pinned/parity-runner-self-test/vm-trace-complete-lifecycle/report.json`：
  4/4 passed、0 difference、0 infrastructure/semantic failure，双端精确观察八类事件。
- M2.7 Complete/Snapshot fresh 证据为
  `target/trace-review/core-four-incarnation-snapshot/report.json`：4/4 passed、0 difference；
  GC state reuse 失败诊断不作为通过证据。
- callback fresh 证据为 `target/compatibility/m2-io-final-pinned/parity-runner-self-test/` 下
  `vm-trace-callback-diff/report.json` 与 `vm-trace-callback-snapshot/report.json`：两种模式
  均 2/2 passed、0 difference、0 infrastructure/semantic failure。
- GC retirement fresh 证据为同目录下
  `vm-trace-state-retirement-cpp/report.json` 与 `vm-trace-state-retirement-rust/report.json`：
  两端均为 6 total opens/5 total closes，其中 captured 为 3 opens/2 GC closes。
- M2.16 CLI fresh 证据为
  `target/compatibility/m217-repl-m0-final/parity-runner-self-test/cli-contract/report.json`：
  18/18 passed、0 failure/infra，binary chunk 明确 deferred 到 M3；
- M2.17 REPL fresh 证据为
  `target/compatibility/m217-repl-m0-final/parity-runner-self-test/repl-contract/report.json`：
  8/8 passed、0 failure/infra，覆盖多行、不完整输入、恢复、stack reset、history、meta 与 completion；
- M2.18 bytecode viewer fresh 证据为
  `target/compatibility/m218-bytecode-m0-final/parity-runner-self-test/bytecode-tool-contract/report.json`：
  15/15 passed，其中 12 个跨语言逐字节 case、3 个 Rust JSON/strict-format case；
- fresh `cargo check --workspace`、workspace 958 tests、all-targets Clippy `-D warnings`、
  warning-free rustdoc 与 fmt 全绿。
- Parser/CodeGen 单元测试只与 raw CLI、structured bytecode 和 publication rollback 证据合并后
  支持 M2.1 的 `completed-local` 判定；不能外推为 M2.2/M2.3 完成。
- 完成条件对应 M1.3、M2.1–M2.3 和 M2 总门槛；目前保持 `partial`。
