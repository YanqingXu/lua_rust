---
status: partial
phase: 5
phase_name: CLI and Tools
last_updated: 2026-07-26
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 5 Report: CLI and Tools

## 结论

**状态：partial。** `lua_app` 和 `lua_bytecode` 是可运行工具；脚本、stdin、
`-e`、`-l`、`-i`、`--`、基础 REPL 以及 text/JSON dump 已有实现。M0
进程 runner、fixture manifest、4-case 双 oracle 和 parity 报告工具已在本地
完成基础设施验收。CLI/tool 语义仍未完整对齐：101 个 non-official 中有
88 个 raw 差异，真实 bytecode 代表用例 2/2 有差异，真实 VM trace 尚无
`--trace-diff` 支持；错误退出合同、JSON 严格性和成熟 REPL 也尚未完成。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | M0 manifest/process runner、4-case 双-oracle differential、bytecode/VM trace parity 报告基础设施与 fail-closed 自检。 |
| `partial` | `lua_app` script/options/basic REPL、`lua_bytecode` text/JSON、真实 bytecode parity。 |
| `not-started` | 两端真实 `--trace-diff` 选项、成熟 REPL 功能、严格 bytecode full/cfg/diff 模式。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Script/CLI option parsing | `crates/lua_app/src/main.rs` | 101-case non-official process artifact；92 个实际执行 case 均记录 stdout/stderr/exit/timeout | partial：语义仍有 88 个 raw 差异，参数组合也未完全覆盖 |
| Stdin and standard streams | `lua_app` stdin source path、stdlib IO handles | 3 个 process tests、101-case fixture gate、双 oracle 4×2 differential 本地均无 infra/timeout failure | 本地修复已验证；见 NOTE-004，远程 CI 待首次运行 |
| REPL | continuation、表达式尝试和安静交互循环 | 手工/smoke 级证据 | partial：history、completion、meta commands、Ctrl-C 等缺失 |
| Bytecode viewer | `crates/lua_bytecode/src/main.rs` 的 text/JSON 输出 | 示例 smoke | partial：错误仍可能以 0 退出，format fallback 过宽，字符串值未输出 |
| Fixture runner | `tests/compatibility/lua_fixtures.json`、`tools/lua_fixture_runner/` | M0 收口时 129 项；当前 131 项全部分类（新增 2 个 M1 raw-byte differential）；101 个 non-official 中执行 92、helper 9、exit 92/92、raw match 4、difference 88、error 0、timeout 0 | M0 infrastructure completed；当前 inventory 的外部 cwd Smoke 复验通过；88 个差异进入 M2 |
| Differential runner | `tests/compatibility/lua51-differential-cases.json`、`tools/run_lua51_differential.ps1` | 官方 Lua 与固定 C++ 两条 lane 均 4/4；官方 `_VERSION` 由 NOTE-001 显式批准 | M0 infrastructure completed；4 个 probe 不等于完整兼容 |
| Bytecode parity runner | `tools/compare_bytecode.ps1` | fail-closed 自检通过；真实代表语料 2/2 生成结构化差异 | infrastructure completed，semantic parity partial |
| VM trace parity runner | `tools/compare_vm_trace.ps1` | synthetic fail-closed 自检通过；真实运行明确报告 unsupported | infrastructure completed，真实 trace capability not-started |

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| 88 个 non-official raw 语义差异 | 92 个执行 case 中 raw match 4、difference 88；显式路径加分析 EOL 规范化后全通道一致 75/92 | M2.4–M2.16 |
| focused differential 只覆盖最小合同 | 双 oracle 均 4/4；GC case 只覆盖一次 weak-value observable，不证明真实 GC 闭环 | NOTE-002、M1.7–M1.13 |
| 真实 bytecode parity 尚未通过 | 代表性真实 corpus 2/2 有结构化差异 | M2.2、M2.3、M2.18 |
| 真实 VM trace 尚不受支持 | runner 自检通过且 fail-closed，但两端缺少对应 `--trace-diff` 输出合同 | M2.7、M2.16 |
| CLI 细节未全量对齐 | hash-bang、binary input、source name、错误类别和所有参数组合缺少进程矩阵 | M2.16、[NOTE-007](deviation_log.md#note-007-lua-字符串尚未采用任意字节表示) |
| REPL 只提供基础循环 | 无 history/completion/meta command/prompt/Ctrl-C 验收 | M2.17 |
| `lua_bytecode` 输出合同不严格 | 缺文件/compile error 只打印后返回；未知 format 回落到 text；JSON 手写转义；字符串常量不显示内容 | M2.18 |
| Binary chunk 不受支持 | viewer 不构成 binary chunk 支持；M1.6 后 `string.dump` 会明确报 unsupported | [NOTE-003](deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk)，M3.1–M3.3 |

## Oracle 与验收

- CLI/REPL/工具的 stdout、stderr、exit status、timeout、JSON 和 trace 比较官方
  Lua 5.1.5 与 `lua_cpp@87c15e6`；项目专有选项以固定 C++ oracle 为准。
- 本地 4-case 证据摘要见
  [initial oracle baseline](oracle_baseline_changes/2026-07-26-initial-baseline.md)；
  该结果只关闭最小 probe，不改变本阶段 `partial` 判定。
- 当前本地统一 M0 gate 以 11,765 ms 完成，`passed=true`、
  `hardFailures=0`，保留 non-official、bytecode 和真实 VM trace 共 3 项债务；
  详见 [M0 收口报告](m0_report.md)。
- runner 已通过 fail-closed 自检且无 silent skip；9 个未执行项均有
  `helper` 分类。远程 CI 尚待首次运行。
- M0.3–M0.6 的基础设施门槛已完成；Phase 5 仍需达到 M2.16–M2.18 和 M2
  总门槛，因此整体保持 `partial`。
