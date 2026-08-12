---
status: completed
milestone: M0
last_updated: 2026-08-12
verification_scope: local
remote_ci: pending-first-run
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# M0 收口报告：可信基线与验证闭环

## 结论

M0 的本地基础设施门已完成。2026-08-12 最新完整统一 M0 gate 在 80,581 ms 内返回
`passed=true`、`fullGatePassed=true`、`hardFailures=0`，同时明确保留 1 项开发债务；
原 representative bytecode 债务已由受审计 byte-safe constant/local evidence 和 2/2 零差异报告关闭。这里的
`completed` 表示 oracle、manifest、进程 runner、差分报告和 fail-closed
合同已经可用，不表示 Lua 语义已完整兼容，也不表示 M1–M5 已完成。

远程 CI 尚待首次运行。本报告不能替代首次 workflow 对 checkout、依赖审计、
构建环境和 artifact 上传的验证；若远程环境暴露基础设施故障，对应 M0 项应
重新打开。

## 门禁摘要

| 领域 | 本地结果 | 判定 |
|---|---|---|
| 统一 M0 gate | `passed=true`；`fullGatePassed=true`；`hardFailures=0`；1 debt；80,581 ms | M0 infrastructure passed |
| Rust quality | fresh M1 重验的 fmt、all-targets Clippy、Debug/Release、`-D warnings` rustdoc 全部通过；本机缺少 `cargo-audit`，因此该次 M1 正确 fail closed。最近一次具备 `cargo-audit 0.22.2` 的完整 M1 仍为 passed | code gates passed；audit environment missing |
| Workspace tests | fresh workspace run 当前覆盖 958 项；`cargo check --workspace`、all-targets Clippy `-D warnings`、warning-free rustdoc 与 fmt 同步通过；上一轮 M1 Debug/Release 质量门覆盖 906 项 | passed for changed scope |
| Fixture inventory | M0 收口时 129 = 101 non-official + 24 official + 4 focused differential；当前另增 2 个 M1 raw-byte 与 1 个 M2 unary differential，共 132 | complete；当前 inventory complete-classification 校验通过 |
| Focused differential | 官方 Lua 4/4；固定 C++ 4/4 | passed，`_VERSION` 使用 NOTE-001 |
| Parity runner self-test | bytecode、directory corpus、bytecode viewer 15/15、C++/Rust 真实 Snapshot/Diff producer、Complete/Diff 核心生命周期 4/4、CLI 双实现 18/18、REPL 双实现 8/8，以及 invalid source/empty stdout 负例均通过 fail-closed 自检；Complete 锁定精确八类 evidence、双端非零 exit 与零 infrastructure/semantic failure | passed |
| Remote CI | 尚无首次运行结果 | pending |

本地兼容门使用已经构建并校验的 oracle 可执行文件；最终报告记录了所有可执行
文件的 SHA-256。机器可读入口由
[`oracle.toml`](../../tests/compatibility/oracle.toml) 固定：

- `lua_cpp@87c15e69ceb94eb74e28226ccbefb7e196635711`；
- Lua 5.1.5 source archive SHA-256
  `2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333`；
- official suite archive SHA-256
  `49e4ca6561f82ea605908c5041ab5fad66ed9930fa0686675bd51b02767f18ad`。

本次 fresh gate artifact 为
`target/compatibility/m218-bytecode-m0-final/gate.json`；调用显式绑定
`-CppRoot target/oracles/lua_cpp-source`，避免误用相邻开发工作树。

## Fixture 与进程差分

M0 收口时，[`lua_fixtures.json`](../../tests/compatibility/lua_fixtures.json)
登记 129 个 fixture：125 个原有基线文件全部分类，另有 4 个 focused
differential case。M1 随后加入 2 个 raw-byte differential，M2 再加入 1 个 unary frontend
coverage case，当前 inventory 为 132；complete-classification 校验确认 132/132 均已分类。

101 个 non-official case 的进程级结果如下：

| 指标 | 结果 |
|---|---:|
| selected | 101 |
| executed | 92 |
| helper skipped | 9 |
| exit code 一致 | 92/92 |
| runner status match | 81/92 |
| runner status difference | 11 |
| manifest normalization 后全通道一致 | 82/92 |
| 直接 raw stream/exit 一致 | 78/92 |
| normalization 后仍存在的实现间通道差异 | 10/92（全部只在 stdout） |
| 预期 executable identity raw 差异 | 3/92 |
| runner error | 0 |
| timeout | 0 |
| stderr 一致 | 92/92 |
| 已声明 side effect 一致 | 92/92 |

9 个未执行项都有显式 `helper` 分类，不是 silent skip。81/11 是 runner 的正式
status：`integration.env-metatable-loader` 的 Rust/C++ raw channels 已完全一致，但两边都
以 1 退出，违反 manifest `expected_exit=0`，所以它仍是 difference。除该共同 expected-exit
failure 外，剩余 10 项是 manifest normalization 后仍存在的 stdout 差异。直接 raw 口径
另保留 `runtime.arg-minimal/negative/simple` 的 3 项真实 executable path identity；本轮没有
通过 normalization 隐藏任何未登记差异。

进程 runner 入口为
[`tools/lua_fixture_runner`](../../tools/lua_fixture_runner/README.md)，本地
机器 fresh 报告位于
`target/compatibility/m218-bytecode-m0-final/non-official.json`。

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
unsupported、timeout、空 bytecode stdout 和结构化差异不会静默成功。invalid source 负例
必须非零退出并留下完整 `infrastructure-failed` 报告。

当前 parity 状态：

| 债务 | 当前证据 | 后续 |
|---|---|---|
| Representative bytecode | M0 代表集 2/2 passed；后续 M2 closure 1/1、integration corpus 5/5、fresh full-valid corpus 131/131，均为 0 differences、0 infrastructure failure；C++ schema-2 byte-safe constant/local sidecar fail-closed 合并并通过 embedded-NUL 自检 | 131-case manifest 覆盖全部 24 个 vendored official 文件与固定 C++ 的 36/36 frontend-produced opcode；M2.1 limit/error matrix 与 M2.18 viewer 15/15 均 completed-local |
| Real VM trace | `lua_app --trace file`/`--trace-diff file` 输出 JSONL Snapshot 或 post-instruction changed-register；跨实现 Snapshot/Basic、Diff/Basic 代表集均为 12/12；Complete/Diff 核心生命周期 4/4 已进入 fail-closed 自检，独立 Snapshot/Complete 核心 4/4 也零差异；metamethod/debug-hook callback Snapshot/Diff 双 2-case 同样零差异并进入 runner；C++/Rust GC retirement producer 各自锁定 3 open/2 close | M2.7 completed-local；更广 M2.3 trace corpus 仍开放 |

M0.6 的完成标准是可靠生成报告和 fail closed，不是提前要求 M2 语义全绿。

## 当前门禁债务与 M2.7 跟踪边界

统一 gate 只登记 `non-official-semantic-differences`：11 runner status differences、
81 status matches、9 helpers skipped；`stdlib.iolib` 已因 text seek/tell 与 `io.popen`
对齐移入 match，另 10 项仍有 normalization 后 stdout 差异。

旧 `complete-vm-trace-profile-incomplete` 已由核心 Diff/Complete 4/4 移除。该关闭不能外推为
M2.7 的本地 trace lifecycle 合同已经完成：metamethod/debug-hook callback 的事件顺序、
`stackTop`、CALL depth 与 changed-register 已双模式零差异；Rust 以 `StateHandle`
generation 处理 state retirement，固定 C++ adapter 则使用 allocator 的精确析构通知，在
下一个安全 trace 边界发布 close 并清除旧 state cache。双端 fail-closed producer 自检均要求
3 次 captured open 与前两次 GC 的 2 次 close；不再依赖地址复用猜测。

M2.16 CLI 合同也已进入同一 self-test：
`target/compatibility/m217-repl-m0-final/parity-runner-self-test/cli-contract/report.json`
为 18/18 passed，逐例锁定参数、stdin、`-e/-l/-i/--`、script args、hash-bang、文件错误、
source name、stdout/stderr/exit 与 REPL error recovery。binary chunk execution 明确 deferred
到 M3，不计作 CLI 假通过。

M2.17 REPL 合同同样进入 hard self-test：
`target/compatibility/m217-repl-m0-final/parity-runner-self-test/repl-contract/report.json`
为 8/8 passed，锁定 banner/help/history/goodbye、多行与不完整输入、错误恢复、裸名称
stack reset、表达式、`.bytecode/.ast/.gc`、meta 错误及 completion。

M2.18 bytecode viewer 合同位于
`target/compatibility/m218-bytecode-m0-final/parity-runner-self-test/bytecode-tool-contract/report.json`：
15/15 passed，覆盖参数/退出码、compact/full/cfg/diff、byte-safe string、nested Proto、
文件/编译错误以及 schema-2 JSON/strict format。

原第 2 项 `representative-bytecode-differences` 已由
`target/compatibility/bytecode-representative/report.json` 的 2/2 passed、0 difference
关闭；这不表示 nested compiler parity 完成。上述 M2.7 广域问题仍是后续输入，但不是
infrastructure hard failure。任何未来报告都不得
通过删 case、扩大 normalization 或忽略 unsupported 来减少债务。

## M1 交接

M1 已成为当前 active milestone，优先顺序是：

1. 按 [ByteString RFC](byte_string_rfc.md) 完成 M1.1–M1.3；
2. 并行冻结 Runtime owner、coroutine handle 和 root inventory；
3. 在 deterministic shutdown 与所有权闭环前不启用真实 sweep；
4. 每次基础层迁移后重跑统一 M0 gate，确保 0 hard failure 且债务变化可解释。

详细执行台账和后续依赖见 [plan.md](../../plan.md)；已知兼容边界见
[deviation log](deviation_log.md)。
