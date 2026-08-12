---
status: partial
phase: 5
phase_name: CLI and Tools
last_updated: 2026-08-12
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 5 Report: CLI and Tools

## 结论

**状态：partial。** `lua_app` 和 `lua_bytecode` 是可运行工具；脚本、stdin、
`-e`、`-l`、`-i`、`--`、完整本地 REPL 合同以及 text/JSON dump 已有实现。M0
进程 runner、fixture manifest、4-case 双 oracle 和 parity 报告工具已在本地
完成基础设施验收。CLI/tool 语义仍未完整对齐：101 个 non-official 中执行 92 个，
runner status 为 81 match/11 difference；manifest normalization 后 82/92 全通道一致，
直接 raw stream/exit 为 78/92 一致。尚有 11 个 normalization 后的 stdout 差异，另 3 个
raw 差异是已登记的 executable identity。131 个有效源码的 structured bytecode 已全部零差异，
M2.1 compiler limit/error matrix 已完成本地对齐；`lua_app --trace file` 与固定 C++ evidence
adapter 已支持真实 Snapshot/Basic 与 Diff/Basic parity，两种模式在同一 12-case 代表集均为
零差异，`stackTop`/`changedRegisters` 已进入 M0 硬门；真实 CALL/RETURN 与顶层 error
value/category 专项也已对齐。Complete 核心生命周期、M2.17 REPL 与 M2.18 bytecode viewer
已完成本地合同；更广 M2.3 trace corpus 与其余语义差异仍未完成。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | M0 manifest/process runner、4-case 双-oracle differential、bytecode/VM trace parity 报告基础设施与 fail-closed 自检。 |
| `completed-local` | `lua_app` CLI 双实现 18/18、REPL 双实现 8/8、`lua_bytecode` viewer 15/15、Complete 核心生命周期双模式与 GC retirement producer 合同。 |
| `partial` | 更广 VM trace corpus与剩余 non-official 语义差异。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Script/CLI option parsing | `crates/lua_app/src/main.rs` | 双实现 CLI contract 18/18；101-case non-official process artifact 的 92 个执行 case 均记录 stdout/stderr/exit/timeout | completed-local；剩余 11 项属于程序语义/输出，不是参数解析缺口 |
| Stdin and standard streams | `lua_app` stdin source path、stdlib IO handles、`lua_stdlib::host_io` | byte-exact process tests、固定 C++ stream/file probes、101-case fixture gate、双 oracle 4×2 differential 本地均无 infra/timeout failure | Windows CRT text output、text seek/tell、`popen` 与文本/二进制文件副作用已验证；远程 CI 仍开放 |
| REPL | `main.rs`、`repl_console.rs`、`repl_meta.rs`、`repl_completion.rs` | 双实现 REPL contract 8/8，固定期望与 raw stdout/stderr/exit 同时校验 | completed-local：多行、不完整输入、恢复、stack reset、history/meta/completion 已锁定；真实终端 Ctrl-C/Tab 保留平台边界 |
| Bytecode viewer | `crates/lua_bytecode/src/main.rs` 的 compact/full/cfg/diff 与 schema-2 JSON | 12 个跨语言逐字节 case + 3 个 Rust JSON/strict-format case；高位 constant raw-byte 回归 | M2.18 completed-local |
| Fixture runner | `tests/compatibility/lua_fixtures.json`、`tools/lua_fixture_runner/` | M0 收口时 129 项；当前 132 项全部分类；101 个 non-official 中执行 92、helper 9、两实现 exit 92/92 一致、status match 80、difference 12、manifest-normalized 全通道一致 81、direct raw stream/exit 一致 78、error 0、timeout 0 | M0 infrastructure completed；当前 inventory complete-classification 校验通过；12 个 status debt 进入 M2 |
| Differential runner | `tests/compatibility/lua51-differential-cases.json`、`tools/run_lua51_differential.ps1` | 官方 Lua 与固定 C++ 两条 lane 均 4/4；官方 `_VERSION` 由 NOTE-001 显式批准 | M0 infrastructure completed；4 个 probe 不等于完整兼容 |
| Bytecode parity runner | `tools/compare_bytecode.ps1`、`tools/test_bytecode_tool_contract.ps1` | 当前仓库全部 131 个有效源码 structured 比较零差异；viewer 15/15 | 当前 corpus、M2.1 limit/error matrix 与 M2.18 viewer completed-local |
| VM trace parity runner | `tools/compare_vm_trace.ps1`、`crates/lua_vm/src/trace.rs`、`tools/cpp_vm_trace_evidence/` | Snapshot/Basic、Diff/Basic 双 12/12；Complete 核心 Snapshot/Diff 双 4/4；callback 双模式 2/2；双 producer retirement 自检 | M2.7 lifecycle completed-local；更广 M2.3 corpus partial |

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| 11 个 non-official status debt | 92 个执行 case 中 status match 81、difference 11；manifest-normalized 全通道一致 82/92、direct raw stream/exit 一致 78/92；10 个 normalization 后差异全部只在 stdout，另 3 个 raw 差异是 executable identity，stderr/side effect 92/92 一致 | M2.4–M2.15 |
| focused differential 只覆盖最小合同 | 双 oracle 均 4/4；GC case 只覆盖一次 weak-value observable，不证明真实 GC 闭环 | NOTE-002、M1.7–M1.13 |
| Binary chunk 不受支持 | viewer 不构成 binary chunk 支持；M1.6 后 `string.dump` 会明确报 unsupported | [NOTE-003](deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk)，M3.1–M3.3 |

## Oracle 与验收

- CLI/REPL/工具的 stdout、stderr、exit status、timeout、JSON 和 trace 比较官方
  Lua 5.1.5 与 `lua_cpp@87c15e6`；项目专有选项以固定 C++ oracle 为准。
- 本地 4-case 证据摘要见
  [initial oracle baseline](oracle_baseline_changes/2026-07-26-initial-baseline.md)；
  该结果只关闭最小 probe，不改变本阶段 `partial` 判定。
- 当前本地统一 M0 gate 以 80,581 ms 完成，`passed=true`、`fullGatePassed=true`、
  `hardFailures=0`，只保留 non-official semantic differences 1 项债务；CLI 18/18、REPL 8/8、viewer 15/15；
  详见 [M0 收口报告](m0_report.md)。
- runner 已通过 fail-closed 自检且无 silent skip；9 个未执行项均有
  `helper` 分类。远程 CI 尚待首次运行。
- M0.3–M0.6 与 M2.16–M2.18 本地工具门槛已完成；Phase 5 仍受剩余语义差异、binary
  chunk/M3 与总计划门槛约束，因此整体保持 `partial`。
