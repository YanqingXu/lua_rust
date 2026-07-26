---
status: completed
milestone: M0
last_updated: 2026-07-26
verification_scope: local
remote_ci: pending-first-run
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# M0 收口报告：可信基线与验证闭环

## 结论

M0 的本地基础设施门已完成。统一 M0 gate 在 11,765 ms 内返回
`passed=true`、`hardFailures=0`，同时明确保留 3 项开发债务。这里的
`completed` 表示 oracle、manifest、进程 runner、差分报告和 fail-closed
合同已经可用，不表示 Lua 语义已完整兼容，也不表示 M1–M5 已完成。

远程 CI 尚待首次运行。本报告不能替代首次 workflow 对 checkout、依赖审计、
构建环境和 artifact 上传的验证；若远程环境暴露基础设施故障，对应 M0 项应
重新打开。

## 门禁摘要

| 领域 | 本地结果 | 判定 |
|---|---|---|
| 统一 M0 gate | `passed=true`；`hardFailures=0`；3 debts；11,765 ms | M0 infrastructure passed |
| Rust quality | `rust_quality_gate.ps1 -SkipAudit` 通过 fmt、all-targets Clippy、Debug/Release 和 `-D warnings` rustdoc | passed locally；audit 未在该命令中执行 |
| Workspace tests | Debug 596/596；Release 596/596 | passed |
| Fixture inventory | M0 收口时 129 = 101 non-official + 24 official + 4 focused differential；当前另增 2 个 M1 raw-byte differential，共 131 | complete；当前 inventory 已通过外部 cwd Smoke 复验 |
| Focused differential | 官方 Lua 4/4；固定 C++ 4/4 | passed，`_VERSION` 使用 NOTE-001 |
| Parity runner self-test | bytecode、directory corpus 和 synthetic VM trace 均通过 fail-closed 自检 | passed |
| Remote CI | 尚无首次运行结果 | pending |

本地兼容门使用已经构建并校验的 oracle 可执行文件；最终报告记录了所有可执行
文件的 SHA-256。机器可读入口由
[`oracle.toml`](../../tests/compatibility/oracle.toml) 固定：

- `lua_cpp@87c15e69ceb94eb74e28226ccbefb7e196635711`；
- Lua 5.1.5 source archive SHA-256
  `2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333`；
- official suite archive SHA-256
  `49e4ca6561f82ea605908c5041ab5fad66ed9930fa0686675bd51b02767f18ad`。

## Fixture 与进程差分

M0 收口时，[`lua_fixtures.json`](../../tests/compatibility/lua_fixtures.json)
登记 129 个 fixture：125 个原有基线文件全部分类，另有 4 个 focused
differential case。M1 随后加入 2 个 raw-byte differential，当前 inventory
为 131；从仓库外传入 `-Root` 的 Smoke 复验确认 131/131 均已分类。

101 个 non-official case 的进程级结果如下：

| 指标 | 结果 |
|---|---:|
| selected | 101 |
| executed | 92 |
| helper skipped | 9 |
| exit code 一致 | 92/92 |
| raw 全通道 match | 4/92 |
| raw difference | 88 |
| runner error | 0 |
| timeout | 0 |
| 显式路径 + 分析 EOL 后全通道一致 | 75/92 |

9 个未执行项都有显式 `helper` 分类，不是 silent skip。最后一项是用于定位债务
的分析指标；正式 artifact 仍保留 raw stdout/stderr/exit 和 88 个差异，没有
通过宽松规范化将其改写为通过。

进程 runner 入口为
[`tools/lua_fixture_runner`](../../tools/lua_fixture_runner/README.md)，本地
机器报告位于 `target/compatibility/non-official.json`。

## 双 oracle focused differential

[`lua51-differential-cases.json`](../../tests/compatibility/lua51-differential-cases.json)
定义 `value-types`、`error-category`、`gc-weak-value` 和 `stderr` 四个
case。[差分 runner](../../tools/run_lua51_differential.ps1) 在两条 lane 上
都通过：

| Lane | Cases | 结果 |
|---|---:|---|
| Rust vs official Lua 5.1.5 | 4/4 | passed |
| Rust vs `lua_cpp@87c15e6` | 4/4 | passed |

官方 Lua 的 `_VERSION` 是 `Lua 5.1`，项目目标值是
`Lua 5.1 (C core prototype)`；该 raw 差异没有被隐藏，而是由
[NOTE-001](deviation_log.md#note-001-项目扩展-_version-值) 显式批准。
四个 probe 只关闭相应最小合同，尤其 `gc-weak-value` 不能证明真实 sweep、
计账、barrier、finalizer 或 shutdown。

## Bytecode 与 VM trace

[bytecode runner](../../tools/compare_bytecode.ps1) 与
[VM trace runner](../../tools/compare_vm_trace.ps1) 已恢复。
[fail-closed 自检](../../tools/test_parity_runners.ps1) 通过，并验证缺失 binary、
unsupported、timeout 和结构化差异不会静默成功。

真实 parity 尚未完成：

| 债务 | 当前证据 | 后续 |
|---|---|---|
| Representative bytecode | 选择 2 个真实 case，0 passed、2 semantic failures、0 infrastructure failure | M2.2、M2.3、M2.18 |
| Real VM trace | synthetic runner 自检通过；真实 `lua_cpp`/`lua_rust` 缺少可比较的 `--trace-diff` 支持 | M2.7、M2.16 |

M0.6 的完成标准是可靠生成报告和 fail closed，不是提前要求 M2 语义全绿。

## 三项保留债务

统一 gate 明确登记：

1. `non-official-semantic-differences`：88 differences、4 raw matches、
   9 helpers skipped；
2. `representative-bytecode-differences`：真实代表语料 2/2 有差异；
3. `real-vm-trace-parity-unsupported`：真实 trace 输出合同尚未实现。

这三项是 M1/M2 的输入，不是 infrastructure hard failure。任何未来报告都不得
通过删 case、扩大 normalization 或忽略 unsupported 来减少债务。

## M1 交接

M1 已成为当前 active milestone，优先顺序是：

1. 按 [ByteString RFC](byte_string_rfc.md) 完成 M1.1–M1.3；
2. 并行冻结 Runtime owner、coroutine handle 和 root inventory；
3. 在 deterministic shutdown 与所有权闭环前不启用真实 sweep；
4. 每次基础层迁移后重跑统一 M0 gate，确保 0 hard failure 且债务变化可解释。

详细执行台账和后续依赖见 [plan.md](../../plan.md)；已知兼容边界见
[deviation log](deviation_log.md)。
