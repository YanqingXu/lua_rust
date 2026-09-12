---
title: lua_rust 完整复刻 lua_cpp 开发计划
status: in-progress
current_milestone: M1
m0_status: completed
last_updated: 2026-09-12
rust_baseline: 503bf99
cpp_oracle: 87c15e6
lua_oracle: Lua 5.1.5
implementation_checkpoint: working-tree@503bf99+audit-remediation
next_primary_task: M2 official db per-frame line hooks, then remaining source failures
---

# lua_rust 完整复刻 lua_cpp 开发计划

## 1. 目标与范围

`lua_rust` 的目标是以 Rust 完整复刻 `lua_cpp` 的可观察逻辑和对外能力。
`lua_cpp` 是项目特有行为的真值来源；Lua 5.1.5 官方实现是标准语言行为的第二真值来源。

“完整复刻”不要求逐行翻译 C++，但要求以下合同全部成立：

1. Lua 源码在两个实现中的返回值、输出、错误、退出状态和副作用一致。
2. 编译器生成的 opcode、操作数、常量、子 Proto 和调试信息一致。
3. VM 的调用帧、upvalue、vararg、多返回、coroutine、metamethod 和错误传播一致。
4. GC 的回收、弱表、finalizer、resurrection、write barrier 和关闭语义一致。
5. Lua 5.1 C API、辅助库、标准库、二进制块和动态模块合同一致。
6. `lua_cpp` 已提供的 sandbox、资源预算、取消、观测和 SDK 能力在 Rust 中有等价实现。
7. 所有保留差异都必须显式登记、获得批准并由测试锁定。

本计划优先保证正确性和可验证性。在行为对齐完成前，不以性能优化、内部重构或增加
非 `lua_cpp` 功能作为主要目标。

## 2. 基线与当前状态

2026-09-12 深度审计以 `503bf99` 为基线，已重新打开 M1 公开访问与 GC mutation
安全合同。**当前工作区修复和验收状态唯一入口为
[审计整改状态](docs/audits/2026-09-12-remediation.md)**。下列 2.1–2.3 保存早期迁移
快照，不可将“已闭环”或测试计数直接作为本轮修复的证明。原始审计见
[风险与复现](docs/audits/2026-09-12.md)。

下一顺序：公开边界与宿主错误 → 表/浮点/参数合同 → M2 具体债务与官方执行、
内存/延迟基线 → 稳定合同上的 M3 ABI 与二进制格式设计。保留 oracle pin，
不使用放宽比较规则来关闭问题。

### 2.1 历史版本基线

| 项目 | 当前基线 | 用途 |
|---|---|---|
| `lua_rust` | `6284135` | 本计划起点 |
| `lua_cpp` | `87c15e6` | 项目行为 oracle |
| 官方 Lua | Lua 5.1.5 | 标准行为 oracle |

`lua_cpp` 基线升级必须通过独立 PR 完成。升级 PR 必须列出新增/变化合同、差分结果和
Rust 侧新增任务，禁止在普通功能 PR 中静默移动 oracle。

### 2.2 历史迁移能力快照

- 6 crate Cargo workspace 已建立。
- Lexer、Parser、AST、CodeGen 和 38 条 opcode 主链已打通。
- VM 已覆盖函数、闭包、upvalue、vararg、多返回、循环、表、主要 metamethod 和 coroutine。
- base、math、string、table、io、os、coroutine、debug、package 已有较宽的函数表面。
- M1.8 shutdown、P1 collector provenance、managed Proto、publication-root
  基础、`TEMPORARY_STATE_ROOTS/PendingState`、Runtime coroutine activation
  trampoline、compiler/stdlib/VM/app publication、production string contract、
  唯一 Heap/service owner、全图 STW、weak/finalizer/resurrection 与
  Lua-visible `collectgarbage("collect"/"step")`、任意 Lua
  `pcall/xpcall`、`tostring/__tostring`、多轮 `load` reader、`table.sort`
  comparator、debug hook、VM opcode metamethod、`dofile`、`table.foreach[i]`、package 与
  `string.gsub` Runtime continuation、main-state StateArena ownership 与显式
  IO resource drain、通用 non-string scoped-access 与 compiler limit/error parity
  全生产路径清零完成后，958 个
  workspace tests 已枚举；
  当前 `cargo check --workspace`、fmt、workspace tests、all-targets Clippy、warning-free rustdoc、24/24 root inventory、
  8/8 mutation inventory、17-path string contract 与 54-path heap contract
  已通过，完整质量门结果见
  本节最新续接记录。
- fixture manifest 当前共 132 项：101 个 non-official、24 个 official 和
  7 个 differential（4 个 M0 focused cases、2 个 M1 raw-byte cases，加 1 个 M2 unary case）。
- 101 个 non-official 中执行 92 个、跳过 9 个 helper；92/92 两实现退出码一致，
  runner status match 81/92、difference 11、runner error 0、timeout 0。`stdlib.iolib`
  已因 text seek/tell 与 `io.popen` 对齐移入 match；manifest 声明 normalization 后
  82/92 全通道一致，direct raw stream/exit 仍为 78/92；
  `integration.env-metatable-loader` 是额外一个 raw 全通道一致但两实现
  都违反 manifest `expected_exit=0` 的 case。剩余 14 个 raw stream 差异中 3 个只是真实
  executable path identity，另 11 个实现间差异全部在 stdout；stderr 与已声明文件副作用
  为 92/92 一致。
- 4 个 focused differential case 在官方 Lua 5.1.5 和
  `lua_cpp@87c15e6` 两条 lane 上均为 4/4；stock `_VERSION` 差异显式引用
  `NOTE-001`。
- 2026-08-12 fresh 本地统一 M0 gate 为 `passed=true`、`fullGatePassed=true`、
  `hardFailures=0`、1 项债务，用时 80,581 ms；CLI 双实现合同为 18/18、REPL 为 8/8，新增 bytecode viewer 合同为 15/15，真实 VM Snapshot/Basic 与 Diff/Basic
  代表集均为 12/12，Complete/Diff 核心 4-case 为 4/4、0 difference、0 infrastructure
  failure，并在双端观察 instruction/call/return/yield/resume/upvalue-open/upvalue-close/error
  八类 required evidence。独立 Complete/Snapshot 核心 4-case 也为 4/4 零差异。该结果关闭
  核心生命周期切片；随后 metamethod/debug-hook callback 的 Diff/Snapshot 2-case 也已零差异。
  C++ evidence adapter 现通过 allocator 析构通知精确发布 GC retirement close，并与 Rust
  各自通过 3-open/2-GC-close fail-closed 自检。fresh M1 重验的静态合同、fmt、workspace
  Clippy、Debug/Release、rustdoc
  与 raw-byte differential 全部通过；本机当前缺少 `cargo-audit`，因此该次 gate 正确返回
  `foundationPassed=false`、1 hard failure。最近一次具备 `cargo-audit 0.22.2` 的完整 M1
  仍为 `foundationPassed=true`、1 项跨平台/allocator 债务、73,578 ms；远程 CI 尚待首次运行。

### 2.3 历史阻塞快照（当前状态见本节开头）

| 领域 | 当前问题 | 优先级 |
|---|---|---|
| 验证 | M0 runner 已建立且 fail-closed；当前 non-official 有 11 个 runner status debt：2 个 userdata print、7 个 GC observable、1 个 arithmetic/error provenance、1 个双方共同 expected-exit failure | P0 |
| CI | 本地统一质量/M0 门已通过，远程 workflow 尚待首次运行 | P1 |
| 字符串 | ByteString/GcString/StringPool、编译器和宿主边界已迁移 arbitrary bytes；当前生产构造强制 canonical interning，`Value::String` Eq/Hash 使用身份，内容语义走 collector/state-scoped bytes 并受静态合同门保护；binary chunk 与未来 C API 仍待后续里程碑 | P0 |
| GC | Runtime safe-point full STW、五阶段 explicit incremental collector 与 allocation-triggered automatic cycle 已真实回收 weak/strong 全图、交付 protected finalizer 并更新 object/accounted/managed-allocator 计账；VM `collectgarbage("collect"/"step")`、`stop/restart/setpause/setstepmul` 与 `gcinfo/count` 已接线 | P0 |
| 生命周期 | Runtime/StateArena 已有确定性 Lua `__gc`→显式 IO resource→arena-owned main/coroutine state→Thread→ordinary→fixed 销毁与 1000 轮归零测试；唯一 Heap/HeapId、scoped service context、standalone Drop reclaim、main-state arena owner、弱 ObjectId 服务注册表与 allocator live=0 已闭环。`package.loadlib` 未实现，当前无动态 module handle | P0 |
| GC 引用安全 | `GcRef` 已携带进程级不复用 `ObjectId`，collector live table 在解引用前校验地址、身份与类型；managed Proto、checked open-Upvalue owner、临时对象/状态根、publication、canonical/scoped string、fixed/pending-finalizer roots、Runtime canonical tracer、不可达 state prepass 与 explicit/automatic collection 已落地。`LuaState::with_gc_ref<T>` 与显式 collector scoped API 已覆盖 stdlib 和 VM dispatch，生产直接 `GcRef::{as_ref,as_mut}` 已清零并有全源静态守卫；`with_vm_context_parts` 已删除，显式 collector context 隐藏动态 collector、保留独立 StringPool，普通 C callback 通过嵌套 context 恢复动态服务。全量 stdlib Miri 23/23 与两项 VM 边界 Miri 均通过 | P0 |
| GC 安全 | 8-family production mutation inventory、checked post-write barrier、active-allocation publication、动态容器容量对账、活动 CallInfo 窗口扫描与 allocation-triggered Runtime safe point 已落地；automatic 周期在单一安全点完成，显式 bounded step 周期保持独立所有权 | P0 |
| IO | raw stdin/stdout/stderr、Windows CRT text-mode EOL/seek/tell、文本/二进制文件副作用、tmpfile、`popen` read/write/close 与显式 shutdown drain 已通过 byte/process/资源测试；未来 M4 sandbox capability 需把当前 unrestricted process 默认收敛成配置合同 | P1 |
| Bytecode parity | schema v2 与固定 C++ byte-safe constant/local evidence sidecar 已补齐 constant/sub-Proto/function/line/local/upvalue 证据，并对 embedded NUL fail-closed 自检；当前仓库除唯一刻意非法语法负例外的 131/131 个 Lua 源码均为完整 structured 零差异，共覆盖全部 24 个 vendored official 文件与 36/38 opcode，即固定 C++ 合同中的 36/36 frontend-produced opcode；`NOT/TESTSET` 明确 intentionally-not-produced。M2.1 的递归/嵌套、locals/upvalues、非法 lvalue、malformed method、超长跳转、300-register 与 262145-constant 合同已完成固定 C++ parity；M2.18 viewer 的参数/退出码、byte-safe text/JSON、nested Proto、full/cfg/diff 已由 15/15 进程合同关闭 | P0 |
| VM trace parity | `lua_app --trace file`/`--trace-diff file` 与固定 C++ trace evidence adapter 已接入；真实 Snapshot/Basic、Diff/Basic 代表集均为 12/12。Complete 核心双模式 4-case 与 callback 双模式 2-case 均零差异。Rust 以 `StateHandle` generation 交付 retirement；C++ adapter 通过 allocator 析构通知发布 close 并清理 state cache，双端 producer lifecycle 自检均锁定 3 open/2 GC close。M2.7 completed-local，更广 M2.3 trace corpus 仍开放 | P0 |
| C API | 无 `lua_capi`、公开头、staticlib/cdylib 和 123 项官方 API 合同 | P1 |
| 二进制块 | 旧进程内 registry 已删除；`string.dump` 在真实 serializer 前显式返回 unsupported，尚无可持久化 binary chunk | P1 |
| 动态模块 | `package.loadlib` 明确不支持动态库 | P1 |
| 生产能力 | sandbox、预算、取消、owner-thread、metrics、worker 尚未迁移 | P2 |
| 文档 | M0/Phase 1–5 报告与 11 项 deviation 已建立；后续随证据持续更新 | P1 |

## 3. 执行原则

### 3.1 双 oracle

- Lua 5.1 标准行为：同时比较官方 Lua 5.1.5、`lua_cpp` 和 `lua_rust`。
- `lua_cpp` 扩展行为：以固定 SHA 的 `lua_cpp` 为准。
- 两个 oracle 不一致时，先建立最小复现，再由维护者决定目标行为。
- 决定结果必须进入 `docs/rust_migration/deviation_log.md`。

### 3.2 验证先于功能

每项迁移必须先具备失败测试或差分用例。不得仅凭函数名、代码结构或内部单元测试宣布完成。

### 3.3 字节模型先于 C API

在 Lua 字符串仍由不一致的 UTF-8/Latin-1 shim 表示时，不开始正式 C API、binary chunk
和动态模块工作，避免围绕错误 ABI 返工。

### 3.4 所有权先于 sweep

在 coroutine state、dump Proto、VM root set 和 shutdown 所有权闭环完成前，不把真实
sweep 直接接入现有 VM。否则当前的内存泄漏可能变成悬垂指针和 use-after-free。

### 3.5 禁止静默近似

占位实现、固定返回值、模拟计数和 unsupported 分支必须满足至少一项：

- 转换为显式 `NotImplemented`/feature gate；
- 进入 deviation log；
- 有明确任务 ID 和阻塞测试；
- 在兼容报告中标为 expected failure。

不得用只验证同一个模拟实现的测试作为兼容完成证据。

## 4. 总体里程碑

```text
M0 可信基线与验证闭环
  ↓
M1 字节字符串、所有权与真实 GC
  ↓
M2 Lua 源码、标准库与工具行为对齐
  ↓
M3 Binary Chunk、C API 与动态模块
  ↓
M4 生产运行时、安全与观测
  ↓
M5 跨平台 SDK、性能与发布
```

M0 是所有工作的共同前置。M1 完成后，M2 的源码兼容和 M3 的 C API 边界可以有限并行；
M4 必须建立在 M1 和 M3 的生命周期、allocator 和公开状态合同之上。

### 4.1 里程碑执行状态（2026-09-12）

| 里程碑 | 状态 | 当前证据或入口 |
|---|---|---|
| M0 | `completed-with-known-debt` | 固定 case/渠道/签名策略、doctests 和官方 source lane 已加入；REPL completion 漏迁移修正后最终完整 M0 为 0 hard failure，9 个具体 non-official case debt 保留。见[整改状态](docs/audits/2026-09-12-remediation.md)。 |
| M1 | `reopened-safety` | 公开 Runtime/GC、callback/栈窗口及普通 Lua 迭代调用通过定向默认 Miri；递归 Debug/Release 和 1/2/8 MiB 宿主线程栈通过，完整 M0/M1 基础门复验通过。公开 allocator、自动 GC 暂停上界与完整资源预算仍开放。 |
| M2 | `active-limited` | 本轮 non-official 已为 83 match / 9 具体 debt；数字显示和普通 arithmetic provenance 已收口。官方 source 快照 11/19 完成、5 失败、3 超时，保留具体根因与双 oracle 冲突。已有 bytecode/trace 历史切片保持独立证据。 |
| M3 | `design-draft` | 已核实 stock 与固定 C++ 的不同 dump payload，形成[格式、verifier、ABI 与模块生存期设计](docs/rust_migration/m3_contract_design.md)。实现等待前置风险收口。 |
| M4 | `pending` | 预算、取消、公开 allocator 与 capability 合同仍待实现；记账不能替代可恢复 OOM。 |
| M5 | `pending` | 等待兼容性与生产运行时门槛。 |

2026-09-01 提交 `503bf99` 的远程 CI 已成功；后续审计整改不能沿用该结果
宣布新修复通过远程 CI。历史 M0 completed 表示验证基础设施达到当时门槛，不代表
全部语义债务关闭。当前完整 gate、局部 smoke、已知失败及显式排除项分开报告。

## 5. M0：可信基线与验证闭环

### 5.1 M0 目标

建立可以持续回答“Rust 与 C++ 还差什么”的自动化系统，修复当前 CI 自身的确定性故障。

### 5.2 任务清单

#### M0.1 固定 oracle

- 新建机器可读的 oracle 配置，建议为 `tests/compatibility/oracle.toml`。
- 记录：
  - C++ 仓库 URL 和 commit SHA；
  - 官方 Lua 5.1.5 来源及 SHA-256；
  - official suite 文件清单和 SHA-256；
  - 差分输出 schema 版本。
- CI 使用第二次 checkout 将 `lua_cpp` 固定到指定 SHA，禁止使用浮动 `main`。
- oracle 更新必须生成差分摘要。

验收：

- 任意 CI run 都能准确报告所用 C++ SHA 和 Lua oracle hash。
- 修改 oracle SHA 会触发专门的 baseline-change 检查。

#### M0.2 修复基础质量门

- 删除 `.cargo/config.toml` 中递归的 `nextest` alias，或改名为 `nt`。
- 在 nextest 修复前，以 `cargo test --workspace` 作为可靠 fallback。
- Clippy 改为：

  ```powershell
  cargo clippy --workspace --all-targets -- -D warnings
  ```

- 清理当前 104 条 test-target Clippy 诊断。
- 文档检查增加：

  ```powershell
  $env:RUSTDOCFLAGS = "-D warnings"
  cargo doc --workspace --no-deps
  ```

- 增加 Release 测试。
- 避免 workflow 逐项运行后再重复执行完整质量脚本。
- 区分 `cargo audit` 工具缺失、网络失败和真实漏洞。

验收：

- 本地质量脚本和 CI 使用同一入口。
- Debug/Release、fmt、all-targets clippy、doc 全部通过。
- CI 中不存在被标记为成功但实际未执行的步骤。

#### M0.3 Lua fixture manifest

- 为 `tests/lua` 建立 manifest。
- 每个文件至少登记：
  - `id`；
  - 相对路径；
  - 类型：`entry`、`helper`、`negative`、`manual-output`；
  - 所需工作目录；
  - 所需参数和环境；
  - 预期退出状态；
  - timeout；
  - 是否比较 stdout/stderr；
  - 是否允许非确定字段；
  - 适用 oracle。
- 不直接逐文件执行 helper/module 文件。
- 将 `manual-output` 脚本逐步转换为：
  - 明确 assert；
  - golden 输出；
  - 结构化差分。

验收：

- 125 个现有脚本全部进入 manifest，不能存在未分类文件。
- runner 能正确区分预期语法错误和真实失败。

#### M0.4 进程级 CLI runner

- 为 `lua_app` 建立真正的子进程测试。
- 捕获并比较：
  - stdout bytes；
  - stderr bytes；
  - exit code；
  - timeout；
  - 输出文件和其他显式副作用。
- 支持路径、地址、临时目录等非确定字段的受控规范化。
- 禁止无规则地忽略整行或整段错误。
- 首先覆盖：
  - basic；
  - control_flow；
  - functions；
  - regressions；
  - runtime；
  - stdlib；
  - tables。

验收：

- 当前 101 个非 official 脚本的结果形成机器可读 JSON artifact。
- 每个差异都有 case ID、C++ 结果、Rust 结果和 diff。

#### M0.5 恢复标准差分

- 从 C++ 迁入 4 个 differential cases：
  - `value-types`；
  - `error-category`；
  - `gc-weak-value`；
  - `stderr`。
- 建立两条 lane：
  - Rust vs 官方 Lua 5.1.5；
  - Rust vs 固定 SHA 的 `lua_cpp`。
- 先修复宿主 stdin/stdout/stderr，使 runner 能真实观察结果。
- 将 `_VERSION`、错误类别、弱表可观察副作用纳入 schema。

验收：

- 4 个用例能够被执行和比较，不允许因空输出、skip 或 runner 错误假绿。
- 第一阶段允许语义失败，但基础设施必须稳定生成报告。
- M2 完成条件要求 4/4 通过或存在已批准 deviation。

#### M0.6 恢复 bytecode 与 VM trace 差分

- 从 `012b449^` 恢复并更新：
  - `tools/compare_bytecode.ps1`；
  - `tools/compare_vm_trace.ps1`。
- bytecode 比较至少包括：
  - 38 opcode 序列；
  - 32-bit instruction word；
  - A/B/C/Bx/sBx；
  - RK operand；
  - 常量值及顺序；
  - 子 Proto；
  - 参数、vararg、max stack；
  - line info、local、upvalue names。
- VM trace 比较至少包括：
  - pc/opcode；
  - 活跃调用帧；
  - stack top；
  - changed registers；
  - upvalue open/close；
  - call/return/yield/resume；
  - 错误值和错误类别。

验收：

- 每次失败都保存最小输入和结构化 diff。
- PR 快速门运行代表性语料；nightly 运行完整语料。

#### M0.7 文档和状态治理

- 将 README 的 Phase 1–5 状态改为证据驱动。
- 建立：
  - `phase_1_report.md`；
  - `phase_2_report.md`；
  - `phase_3_report.md`；
  - `phase_4_report.md`；
  - `phase_5_report.md`。
- 将当前已知近似全部写入 deviation log：
  - 模拟 GC；
  - 非标准 dump；
  - memory stdio；
  - dynamic library unsupported；
  - OS/locale/time 近似；
  - Unicode/byte-string 差异；
  - C API 缺失。

验收：

- README、phase report、代码和 CI 之间不存在已知状态矛盾。
- 每个 expected failure 都能反向查到 deviation 或任务 ID。

### 5.3 M0 完成门槛

- 质量门入口稳定通过。
- 125 个 Lua 文件全部有 manifest。
- 进程级 CLI、4-case differential、bytecode 和 VM trace runner 均能生成可靠报告。
- oracle 被固定且可审计。
- 所有当前已知近似进入 deviation log。

### 5.4 M0 执行台账（2026-07-26）

| 任务 | 状态 | 验证证据 |
|---|---|---|
| M0.1 固定 oracle | `completed` | `oracle.toml`、Lua/official source manifest、SHA 校验和 baseline-change policy 均已接入；C++ 固定为 `87c15e6`。 |
| M0.2 基础质量门 | `completed` | 本地 `rust_quality_gate.ps1 -SkipAudit` 通过 fmt、all-targets Clippy、596 个 Debug/Release workspace tests 和 `-D warnings` rustdoc；本地未运行 audit，远程 CI 尚待首次运行。 |
| M0.3 fixture manifest | `completed` | 当前 132 项全部分类：101 non-official、24 official、7 differential；其中 125 项为原有基线，另有 4 个 M0 focused、2 个 M1 raw-byte 与 1 个 M2 unary differential。 |
| M0.4 进程级 CLI runner | `completed` | 101 个 non-official 中执行 92、跳过 9 个 helper；92/92 两实现 exit 一致，status match 80、difference 12、error 0、timeout 0；manifest-normalized 全通道一致 81/92、direct raw stream/exit 一致 78/92、side effect 92/92 一致。 |
| M0.5 标准差分 | `completed` | `value-types`、`error-category`、`gc-weak-value`、`stderr` 在官方 Lua 和固定 C++ 两条 lane 上均 4/4 通过；官方 `_VERSION` 差异由 NOTE-001 批准。 |
| M0.6 bytecode/VM trace 差分 | `completed`（基础设施） | 两个 parity 工具已恢复且 fail-closed 自检通过；当前仓库有效源码 structured bytecode 131/131 零差异；真实 VM Snapshot/Basic 与 Diff/Basic 代表集均为 12/12 并进入完整 M0 硬门；Complete/Diff 核心 4-case 也以精确八类 evidence、双端 exit=1 和零差异进入 fail-closed 自检，独立 Snapshot/Complete 核心 4-case 同样零差异。更广的 callback `stackTop` 和 C++ adapter GC state identity 继续作为 M2.7 开发债务，不属于 M0 基础设施 hard failure。 |
| M0.7 文档与状态治理 | `completed` | README、M0/Phase 1–5 报告与 NOTE-001–010 已形成证据链；NOTE-010 记录进入 M1 后的 hash 决策。 |

统一 M0 gate 的 2026-08-12 fresh 本地结果为 `passed=true`、`fullGatePassed=true`、
`hardFailures=0`、1 项债务，`durationMs=48798`。唯一 gate debt 是 non-official 12 个 runner
status differences；Complete 核心 trace 债务已关闭。callback `stackTop` 与固定 C++ adapter
GC state identity 仍作为 M2.7 广域开发债务保留，不会被核心 manifest 的通过结果掩盖。详细数据和验证边界见
[M0 收口报告](docs/rust_migration/m0_report.md)。

## 6. M1：字节字符串、所有权与真实 GC

### 6.1 M1 目标

建立可支撑真实回收、binary chunk 和 C API 的安全运行时基础。

### 6.2 字节字符串迁移

#### M1.1 ByteString 设计

- 编写短 RFC，明确：
  - Lua 字符串是任意字节；
  - 内部不可假设 UTF-8；
  - NUL 合法；
  - hash、equality、length 全部按 bytes；
  - CLI 显示与错误显示才进行 UTF-8/lossy 转换；
  - 文件路径和 Lua 字符串是不同类型。
- 推荐表示：

  ```rust
  Box<[u8]>
  ```

  或具有同等不可变、稳定地址和精确长度语义的容器。

#### M1.2 迁移 GcString 与 StringPool

- `GcString::data()` 返回字节切片。
- `as_ptr + len` 指向同一真实缓冲。
- StringPool 使用字节 key。
- hash 与 `lua_cpp`/Lua 5.1 对齐。
- 删除 Latin-1/UTF-8 多套转换 shim。
- 增加：
  - NUL；
  - 0x00–0xFF；
  - 无效 UTF-8；
  - 中文 UTF-8 原始字节；
  - 长字符串；
  - interning identity 测试。

#### M1.3 迁移编译器和宿主边界

- Lexer/Parser 输入改为 bytes 或明确的 byte cursor。
- Lua 语法 token 可按 ASCII 识别，字符串字面量内容必须原样保留。
- `load`、`loadfile`、`loadstring`、CLI stdin/file 都使用统一字节入口。
- string/io/package/os/dump/bytecode 工具统一使用 ByteString。
- 所有文本展示集中到单独 formatting 层。

验收：

- 任意字节字符串可以创建、比较、拼接、查找、dump/load 和经 C API 读取。
- UTF-8 脚本不再双重编码。
- `#string.char(255)`、hash、pointer+length 同时正确。

### 6.3 运行时所有权

#### M1.4 Runtime/EngineContext 所有者

- 引入唯一运行时所有者，统一持有：
  - GarbageCollector；
  - StringPool；
  - main LuaState；
  - coroutine state arena；
  - registry/global roots；
  - allocator；
  - IO/services；
  - 后续 execution policy。
- 禁止由多个局部变量通过裸指针拼装隐式生命周期。
- 明确 State、Thread、Proto、Function 和 collector 的销毁顺序。

#### M1.5 Coroutine state 所有权

- 移除 `Box::into_raw(LuaState)` 无释放路径。
- 优先考虑由 Runtime 管理的 generational arena：
  - `Thread` 只保存稳定 handle；
  - Runtime 负责查找和销毁 coroutine state；
  - handle 失效可检测；
  - 不产生 `&'static mut LuaState`。
- 若采用其他设计，必须证明：
  - 单一所有者；
  - 无 mutable alias；
  - Thread 回收时 State 一定释放；
  - debug/coroutine API 不返回超出作用域的引用。

当前 trampoline 细化执行顺序：

1. **已完成本地基座：** 固定 C++/stock 的 `Normal` 祖先重入
   characterization；实现 `Runtime::drive_state_turns`，保证每个 turn 只借
   一个 state、`Switch` 前释放借用、panic 后恢复 slot 与 active count。
2. **已完成生产切片：** 引入 scoped native mailbox/capability 与独立
   `VmExit::NativeRequest`，只让 `coroutine.resume`/`wrap_runner` 发布
   `ResumeRequest`，禁止在 callback 内递归解析/执行 target。生产入口使用
   sealed context/窄能力，不把可 `mem::take` 或转存 raw pointer 的完整
   `&mut LuaState/GC/StringPool` 暴露给 runtime-native callback。
3. **已完成：** C 调用帧进入 deferred 状态并保存 nonce、`func_pos`、wanted result、
   caller CI/PC；Runtime driver 在 caller guard Drop 后再验证和借 target。
4. **已完成：** Runtime 维护 activation frame 栈而非“唯一 active handle 集合”，允许固定
   C++ 的 `A→B→A` `Normal` 祖先激活环；`Running`/`Dead` 仍拒绝。
5. **已完成本地根种子：** transfer args/results/error、Thread handle 与
   deferred frame 进入 Runtime-owned activation buffer，并由 canonical
   root tracer 扫描；live destructive sweep 仍因其他 root/owner 缺口保持禁用。
6. **已完成当前 envelope：** 对齐 caller `Running↔Normal`、
   callee `Running/Suspended/Dead`、
   caller link、`allow_yield`、saved execution count、resume/wrap envelope、
   精确 0/1/多返回与 error identity/traceback。
7. **已完成生产入口迁移：** app、stdlib harness 和 coroutine
   resume/wrap 已由 Runtime 调度；普通生产执行不再通过长生命周期
   `RuntimePartsMut` 绕过调度器。debug 跨 state API 仍是后续项。
8. **部分完成：** 验收包含普通 A→B→C、`Normal` 祖先重入顺序、yield/resume args、
   pcall/C boundary、wrap error、panic/fault cleanup、临时根、数千层链与
   peak borrowed slots 始终为 1；补充“首 turn 成功后 Switch 到
   foreign/stale/borrowed target”及 turn 内 coroutine create 触发 arena
   slot 扩容的回归。当前已锁定精确 `Normal` 祖先输出、protected `pcall`
   resume、wrap yield/error、stdlib coroutine fixtures、root inventory 与
   workspace 回归；数千层链、广义 fault injection 和 debug 跨 state 矩阵仍待补。

#### M1.6 移除悬垂 registry

- 删除 `dump.rs` 中 thread-local `GcRef<Proto>` registry。
- 在真实 serializer 完成前，非标准 dump 必须显式 feature-gate 或返回 unsupported。
- 所有长期缓存必须：
  - 被 GC root；
  - 或保存可验证的 generational handle；
  - 或保存自有序列化数据。

### 6.4 GC 闭环

#### M1.7 Root inventory

- 建立所有 GC root 的机器可读/文档化清单：
  - global/registry；
  - main/coroutine stacks；
  - CallInfo；
  - open upvalues；
  - active function/proto；
  - C closure upvalues；
  - debug hook/error；
  - pending finalizers；
  - temporary protected roots；
  - library-owned live handles。
- 每种 root 都必须有回归测试。

#### M1.8 确定性 shutdown

- Runtime/collector Drop 必须释放所有普通对象和 fixed 对象。
- shutdown 顺序对齐 `lua_cpp`：
  - 拒绝 busy/foreign-thread close；
  - 运行剩余 `__gc`；
  - 清理 native/module/IO resources；
  - 释放 main/coroutine state；
  - 释放 fixed strings/registry；
  - allocator live bytes 归零。
- CLI 和测试不得依赖进程退出替代析构。

#### M1.9 真实 full collection

- `collectgarbage("collect")` 调用真正的 mark、propagate、weak、finalize、sweep。
- `gcinfo` 来自实际内存计账。
- 普通不可达对象必须被销毁。
- 删除当前 `+8/+24` 和固定 step 次数模拟。

#### M1.10 Incremental GC

- 明确 phase：
  - pause；
  - propagate；
  - atomic；
  - sweep；
  - finalize。
- `step` 推进真实工作量。
- `setpause`/`setstepmul` 保存并返回正确旧值。
- 接入 allocation debt/threshold。

#### M1.11 Write barrier

- 接入至少以下写入路径：
  - Table key/value/metatable；
  - Function proto/upvalue/env；
  - Upvalue close/set；
  - Userdata metatable/env；
  - Thread caller/state references；
  - Proto child/constants/debug references；
  - registry/global assignment。
- 增加黑对象指向白对象、增量阶段修改、多 collector/cross-state 的定向测试。

#### M1.12 Weak table、finalizer 与 resurrection

- 对齐：
  - weak key；
  - weak value；
  - ephemeron/键值关系；
  - string 行为；
  - userdata finalizer；
  - finalizer 顺序；
  - resurrection；
  - finalizer 再次触发规则；
  - 关闭时 finalizer drain。

#### M1.13 内存与耐久测试

- 增加：
  - object count；
  - accounted bytes；
  - allocator live/peak bytes；
  - 1000 轮 state create/close；
  - coroutine create/drop；
  - weak/finalizer 多周期；
  - closure/upvalue 生命周期；
  - dump/load 后生命周期。
- 对 unsafe 热点运行可行的 Miri、ASan 或等价检查。

### 6.5 M1 完成门槛

- 无已知 collector、coroutine state 或 fixed object 泄漏。
- `collectgarbage` 真实回收并降低对象数/内存。
- shutdown 后 live objects 和 allocator live bytes 为 0。
- ByteString、root set、barrier、weak/finalizer/resurrection 测试全部通过。
- 不存在返回 `&'static mut` 的运行时裸指针辅助函数。

### 6.6 M1 执行台账（2026-07-29）

| 任务 | 状态 | 当前证据与未闭合项 |
|---|---|---|
| M1.1 ByteString 设计 | `completed-local` | RFC 已冻结 arbitrary bytes、NUL、pointer+length、显式 UTF-8/lossy 边界与 C++ 前向采样 hash。 |
| M1.2 GcString/StringPool | `completed-local` | 单一 ByteString payload、byte-key interning、0x00–0xff/NUL/invalid UTF-8/identity/hash 测试通过，旧 text API 静态扫描为零。 |
| M1.3 编译器和宿主边界 | `partial` | lexer/parser/codegen、string/io/package、CLI source 与 chunk name 已迁移并有 raw-byte 双 oracle case；当前生产 GcString 构造已强制走 StringPool，内容读取已迁移到 collector/state-scoped API。真实 dump/load 与未来 C API 尚未完成。 |
| M1.4 Runtime owner | `completed-local` | pinned `RuntimeStorage` 唯一持有 Heap、StateArena 与 activation service，StateArena 独占 main/coroutine 全部 State Box；Heap 以不复用 HeapId 共同持有 collector、managed allocator ledger 和 canonical StringPool，生产 app/bytecode/compiler/stdlib/VM 无 standalone 构造。LuaState 的 GC/StringPool raw backpointer 已删除，单 state turn 使用 nested/panic-safe 动态 service context；Heap/standalone collector Drop 都回收对象。未来公开 Lua allocator callback 由 M3/M4 跟踪。 |
| M1.5 Coroutine state | `completed-local` | StateArena 独占 main/coroutine 全部 Box；Runtime 仅保存 distinguished main `StateHandle`，shared/direct/turn/root-trace/full/incremental/shutdown 均校验同一 generational slot，collection 不会 sweep main slot，公开 coroutine count 单独计数，shutdown 固定 coroutine slot-order→main。handle identity/generation、retirement 与关闭前 arena 校验已 fail-closed。`PendingState` 以 exact-id `TEMPORARY_STATE_ROOTS` 保护未发布槽，Drop 回滚并推进/退休 generation；create/wrap 在对象临时根内完成 State↔Thread 双向绑定并仅在直接压栈后提交。sealed `RuntimeNativeFunction`、scoped mailbox、独立 VM exit、deferred C frame、Runtime coroutine/protected-call/callback-helper/upvalue/debug-upvalue activation stack、rooted transfer seed 和 generic-for continuation 已接入生产 resume/wrap/debug/`pcall`/`xpcall`/`tostring`/`load`/`table.sort`/`dofile`/`table.foreach[i]`/package/`string.gsub`。open Upvalue 现为 `StateHandle + stack index`，远端 GET/SET 与 `debug.getupvalue/setupvalue` 通过 Runtime 单-state turns 访问；任意 Lua `pcall/xpcall` target/error-handler 与普通 callback helper 都以 typed Runtime activation 持有跨 turn envelope。foreign/stale owner fail-closed，state drain 在 generation 失效前关闭节点；固定 C++ `Normal` 祖先、1000 层 resume/wrap、128 层 nested `pcall`、borrow/panic/fault/activation unwind、main identity/move、shutdown preflight retry 与 coroutine-before-main drain 均有回归。close-time finalizer Runtime-native 重入继续按 M1.12 的明确 fail-closed 边界处理。 |
| M1.6 悬垂 registry | `completed-local` | pseudo dump/source thread-local registry 已删除，`string.dump` 在 M3 serializer 前稳定返回 unsupported。 |
| M1.7 Root inventory | `completed-local` | 24/24 root inventory、8/8 mutation inventory 与 11/11 allocator inventory 校验通过；canonical 双队列、identity-aware collector 队列、managed `ACTIVE_PROTO/DEBUG_PROTO`、checked `OPEN_UPVALUES` owner、temporary object/state roots、activation/debug-upvalue/GC maintenance service、compiler/library/package/IO/VM/app/result publication、pending-finalizer seed、Runtime fixed strings 与不成 root 的 weak resource registry 已接入合同。通用 `LuaState::with_gc_ref<T>` 与显式 collector scoped API 在触碰对象前校验 address/ObjectId/type；`lua_stdlib/src` 与 `lua_vm/src/execute.rs` 的生产直接 managed-object `as_ref/as_mut` 均清零并由全源静态测试锁定。`with_vm_context_parts` 已删除，`with_vm_context_gc` 隐藏动态 collector 但保留独立 StringPool，普通 C callback 以窄嵌套 context 恢复动态服务；全量 stdlib Miri 23/23 与两项 VM 边界 Miri 通过。 |
| M1.8 确定性 shutdown | `completed-local` | Runtime close/Drop 先隔离错误并持续 drain 剩余/新分配 Lua `__gc`，再按注册顺序解析不成 root 的 `ObjectId + static hook` IO resources，隔离 hook error/panic，最后以 coroutine→main state→非 fixed Thread→其余非 fixed→fixed 顺序释放；object/root/string/queue/state/resource-registry/allocator live 均归零。7 layout、fixed/ordinary DropProbe、open-upvalue close、finalizer error/continue、真实文件失败-finalizer后刷盘、preflight retry、registry allocation rollback、hook panic 与 1000-resource/1000-cycle 耐久测试通过。`package.loadlib` 未实现，当前无 native module handle。 |
| M1.9 真实 full collection | `completed-local` | Runtime safe-point STW 消费 canonical tracer，执行 finalizer prepare/resurrection propagation、weak reconciliation、不可达 state prepass 和真实 sweep；Lua `collectgarbage("collect")` 已接线，`gcinfo`/`count` 使用 collector 实际计账。 |
| M1.10 Incremental GC | `completed-local` | collector 持有 pause→propagate→atomic→sweep→finalize、debt/threshold、pause/stepmul 与有界 sweep cursor；Runtime 持久保存 StateHandle/object 双队列，`step` 推进真实预算，大步可完成整轮，周期完成才返回 true；explicit 与 automatic 周期所有权相互隔离。 |
| M1.11 Write barrier | `completed-local` | 8-family mutation inventory 与 fail-closed gate 已落地；`GarbageCollector::with_mut` 统一验证 owner 并执行 post-write barrier，活动周期分配发布初始图，Sweep 期变更保守放弃游标，Proto 保持 construction-only。 |
| M1.12 Weak/finalizer/resurrection | `completed-local` | atomic weak key/value/kv、pending-finalizer weak 语义、protected callback delivery、nested collect 非递归 drain、异常隔离/保留队列、exactly-once、resurrection→再次不可达和 close drain 已有回归。finalizer 内跨 state resume/open-Upvalue 与 close-time Runtime-native 重入当前明确 fail-closed。 |
| M1.13 内存与耐久 | `completed-local-slice` | 共享 managed-payload ledger 覆盖 GC 对象/动态容器、StringPool key、StateArena 与 Runtime resource registry，公开 live/peak/total 快照并在 close 后归零；allocation threshold 在 Runtime 指令边界触发安全 collection。GC object、StringPool key、publication root、StateArena slot、resource registry 五点 one-shot failure injection 与事务性重试已落地，11/11 allocator contract 通过；本地 pinned-nightly Miri 已通过 3 个 core fault cases 和 2 个 Runtime durability cases（含 1000 轮），Windows ASan 已通过相同 5 个 cases 及 1000 轮 coroutine/weak/finalizer/closure-upvalue 组合矩阵，并修复两处实际 alias UB。Linux 验证按当前环境策略延期；binary dump 生命周期等待 M3 serializer。 |

本台账中的 `completed-local` 只表示对应子任务的本地实现与定向证据完成；
远程 CI 首次通过和合入前不改为最终 `completed`。M1 整体仍为 `active`。

## 7. M2：源码、标准库与工具行为对齐

### 7.1 编译器与 Proto

#### M2.1 错误处理

- 将 codegen 中的 `panic!` 转为结构化错误。
- 增加：
  - 超长控制结构；
  - 寄存器耗尽；
  - 递归/嵌套限制；
  - 非法 lvalue；
  - malformed method call；
  - 极端常量/局部变量数量。
- 错误 source、line、category 与 C++ 对齐。

本地状态（2026-08-12）：`completed-local`。codegen 的两个用户可达 panic（超长跳转、匿名
函数内非法 lvalue）和畸形 method AST panic 已收口为 `CodegenError`；子函数失败无论成功与否
都会恢复父 generator 状态并释放 publication roots。递归/嵌套、concat/call 深度、200/201
locals、60/61 upvalues、非法 lvalue、malformed method 的 8 类 CLI 矩阵与固定 C++ raw 完全一致；
五类超长控制流、300-register structured bytecode 和 262145-constant 进程探针也已对齐。固定
C++ 对后两种字段容量采用截断而非报错，Rust 按 oracle 合同保留该行为；通用 production
compiler budget 仍按 M4.2 跟踪。

#### M2.2 Opcode 与 instruction parity

- 逐项验证 38 opcode：
  - enum order；
  - instruction encoding；
  - RK；
  - metadata；
  - jump range；
  - SETLIST；
  - CLOSURE/upvalue binding；
  - CALL/TAILCALL/RETURN；
  - VARARG。
- 每个 opcode 至少有：
  - 单元测试；
  - Lua 行为测试；
  - bytecode differential。

#### M2.3 Proto 完整性

- 对齐：
  - constants；
  - nested protos；
  - num params；
  - is_vararg；
  - max stack；
  - line info；
  - local ranges；
  - upvalue names；
  - source/chunk name。

### 7.2 VM 语义

#### M2.4 调用与返回

- Lua→Lua、Lua→host、host→Lua、递归和重入。
- 多返回的最终/非最终折叠。
- tail call 和 tail return debug event。
- C function error/yield 边界。
- stack overflow 和调用深度限制。

#### M2.5 Closure 与 upvalue

- open upvalue 唯一性。
- sibling closure 共享。
- scope exit、break、return、tailcall 时关闭。
- coroutine yield/resume 穿越 upvalue。

#### M2.6 Metamethod

- `__index`、`__newindex` 链和循环限制。
- 算术、比较、concat、len、call、tostring。
- table/userdata/primitive metatable。
- 错误消息和 source line。

#### M2.7 Coroutine 与 debug

- create/resume/yield/status/running/wrap。
- 首次参数、yield values、resume values、多返回。
- dead/running/error 状态。
- debug hook call/return/line/count。
- getinfo/getlocal/getupvalue/setlocal/setupvalue/traceback。

### 7.3 标准库

#### M2.8 首批可观察修复

- 真实 stdin/stdout/stderr。
- 注册 `_VERSION`。
- 补齐当前与 C++ 注册表相比缺失的函数：
  - `table.pack`；
  - `table.unpack`；
  - `table.move`；
  - `io.popen`；
  - `os.exit`；
  - `os.getenv`；
  - `debug.debug`；
  - `debug.getmetatable`。
- 建立真正的 string metatable，而不是 VM 硬编码 fallback。

#### M2.9 Base 和错误行为

- assert/error/pcall/xpcall。
- load/loadfile/loadstring/dofile。
- getfenv/setfenv。
- collectgarbage/gcinfo。
- tonumber/tostring/select/unpack。
- 错误对象、错误级别、source line 和 traceback。

#### M2.10 String 与 pattern

- 全部按 bytes 工作。
- pattern class、capture、frontier、balanced、gmatch、gsub replacement。
- format 与 Lua 5.1 边界。
- string.dump 在 M3 前显式标记 unsupported，不保留伪兼容。

#### M2.11 Table

- insert/remove/concat/sort。
- comparator 错误和非破坏性失败。
- foreach/foreachi/getn/maxn。
- C++ 项目扩展函数。

#### M2.12 IO

- 默认流和重定向。
- open/read/write/seek/flush/close/lines/type/tmpfile/popen。
- text/binary mode。
- EOF、错误 tuple 和 closed-file 行为。
- file userdata 类型身份和 `__gc`。

#### M2.13 OS

- clock 使用正确的进程 CPU 时间语义。
- date/time 的 local/UTC、`!`、isdst、格式符。
- difftime、execute、exit、getenv、remove、rename、tmpname。
- setlocale 的真实支持或明确、已批准的平台差异。

#### M2.14 Math

- RNG 状态由 Runtime/GlobalState 持有，禁止进程全局共享。
- random/randomseed 与 state isolation。
- 边界参数、NaN、Inf、范围错误和格式。

#### M2.15 Package

- package.config/path/cpath/loaders/preload/loaded。
- require loader 顺序、循环依赖和错误聚合。
- Lua module、module/seeall。
- native loader 在 M3 接入。

### 7.4 CLI、REPL 与 bytecode 工具

#### M2.16 CLI（`completed-local`）

- 参数、stdin、`-e`、`-l`、`-i`、`--`、script args。
- 与 C++ 对齐 `--trace`、`--trace-diff` 等项目选项。
- stdout/stderr/exit code。
- hash-bang、二进制输入、文件错误和 source name。

当前本地合同：除二进制 chunk 执行明确路由到 M3 外，其余条目均由
`tools/test_cli_contract.ps1` 的 C++/Rust 18/18 原始进程证据锁定；文件错误不再泄漏
平台本地化 OS 文本，交互模式会读取 Lua 配置提示符并在错误后继续。

#### M2.17 REPL（`completed-local`）

- 错误后继续会话。
- 多行和不完整输入。
- 自动表达式和前导 `=`。
- history、meta command、completion、prompt、Ctrl-C。
- State/stack 在每次命令后恢复一致。

当前本地合同：`tools/test_repl_contract.ps1` 以固定期望和跨语言原始进程证据锁定
8/8 case，覆盖 banner/help/history/goodbye、多行与不完整输入、语法/运行时错误恢复、
裸名称 stack reset、自动表达式与前导 `=`、`.bytecode/.ast/.gc`、meta 错误和 completion。
Windows 终端路径另实现 Tab、Backspace、Ctrl-C 与 Ctrl-D/Z；自动化 runner 对可重放的
管道输入 fail closed，终端专属按键合同保留在实现与平台验证边界内。

#### M2.18 Bytecode 工具（`completed-local`）

- 错误参数返回非零。
- 严格校验 `--format`。
- text/JSON 正确转义。
- 显示真实字符串常量和 nested Proto。
- full/cfg/diff 模式。
- JSON 使用稳定 schema，禁止手写无效转义。

当前本地合同：固定 C++ 的 compact/full、Mermaid CFG、双文件 diff、参数/退出码与
文件/编译错误由 12 个逐字节跨语言 case 锁定；Rust 的 schema-2 recursive byte-safe JSON
与 strict `--format` 另有 3 个扩展 case。text/diff 对高位 string constant 保留原始 byte，
不会在输出层发生 UTF-8 重编码；15/15 已进入 parity self-test 与 M0 hard validation。

### 7.5 M2 完成门槛

- 125 个项目 Lua fixture 全部按 manifest 得到预期结果。
- 4-case differential 4/4 通过，或仅剩已批准 deviation。
- Lua 5.1 official strict `all.lua` 通过。
- slow suite 通过。
- 38 opcode bytecode/trace 无未解释差异。
- CLI/REPL/bytecode 进程级测试全绿。

## 8. M3：Binary Chunk、C API 与动态模块

### 8.1 Binary Chunk

#### M3.1 格式合同

- 明确目标：
  - Lua 5.1 官方 chunk 兼容；
  - `lua_cpp` 项目扩展；
  - 端序、数字格式和平台字段。
- 建立官方 `luac`、`lua_cpp`、`lua_rust` 三方 golden。

#### M3.2 Serializer/Deserializer

- header；
- function Proto；
- code；
- constants；
- nested protos；
- debug info；
- source；
- upvalue metadata。
- 不允许进程内 ID 或指针进入持久化格式。

#### M3.3 Bytecode verifier

- 长度和整数溢出。
- recursion/depth。
- instruction/opcode/operand 合法性。
- constant/proto/upvalue 索引。
- stack/register 范围。
- allocation/size limit。
- truncated、random、malformed corpus。

验收：

- `string.dump` 结果可跨进程加载。
- 官方/Cpp/Rust 的允许格式按合同互通。
- malformed chunk 不 panic、不越界、不产生未受控分配。

### 8.2 C API

#### M3.4 Crate 与制品

- 新建 `lua_capi` crate。
- 输出：
  - staticlib；
  - cdylib；
  - `lua.h`；
  - `lauxlib.h`；
  - `lualib.h`；
  - 后续 `lua_runtime.h`。
- 所有公开类型使用明确的 `#[repr(C)]`/opaque handle。
- 禁止 Rust panic 穿越 FFI。

#### M3.5 State、stack 与类型 API

- newstate/close。
- allocator callback。
- stack top/index/checkstack/xmove。
- type/query/conversion。
- push/copy/replace/remove/insert。
- registry、globals、pseudo-index。

#### M3.6 Table、userdata、function 与调用

- get/set/raw/table iteration。
- light/full userdata 和 metatable。
- C closure/upvalues。
- call/pcall/cpcall/error。
- reader/writer/load/dump。
- coroutine resume/yield/status。
- GC/debug/hook。

#### M3.7 lauxlib 与 lualib

- buffer API。
- argument checking。
- reference system。
- loadfile/loadbuffer。
- library registration。
- openlibs。

#### M3.8 C API 验证

- 迁移 `lua_cpp` 的官方 123 项机器合同。
- 纯 C 编译测试。
- 独立 C/C++ consumer。
- static/shared link。
- allocator failure、state isolation、protected error。
- API 与官方 Lua 5.1 差分 probe。

### 8.3 Native module

#### M3.9 package.loadlib

- Windows LoadLibrary/GetProcAddress。
- POSIX dlopen/dlsym。
- module registry 和生命周期。
- symbol/error 文本。
- sandbox capability gate。
- unload/State close 顺序。

验收：

- 官方 C API 123/123 通过。
- public export contract 无缺失/多余符号。
- 独立 native module 可构建、加载、调用和卸载。
- 1000 次 module lifecycle 不泄漏。

## 9. M4：生产运行时、安全与观测

### 9.1 Runtime configuration

- 迁移版本化 `RuntimeConfig`。
- unrestricted 默认和 game-server preset。
- 标准库 capability bitmap。
- 文件、进程、动态模块和运行时编译 capability。
- 创建期限制和每次 execution window 限制分离。

### 9.2 执行治理

- instruction budget。
- native work budget。
- monotonic deadline。
- cooperative cancellation。
- finalizer budget。
- compiler/parser/resource limits。
- coroutine 和重入共享同一执行窗口。

### 9.3 Owner-thread 与状态机

- State 固定 owner thread。
- foreign-thread 只能使用生命周期安全 cancellation handle。
- busy/idle/closed 状态明确。
- close、metrics、配置 API 的调用时机有机器合同。

### 9.4 Allocator 与内存

- 支持 Lua allocator callback。
- 明确已路由和未路由的内存范围。
- 失败必须 transactional：
  - 旧块仍有效；
  - 不部分提交 stack/table/string；
  - 可解除限制后继续使用。
- 进程级边界作为最终兜底，不夸大 callback allocator 能力。

### 9.5 Metrics

- 初始/剩余/消耗 instruction budget。
- native work。
- deadline 和 cancellation。
- stop reason。
- allocator live/peak。
- GC phase/objects/bytes。
- 只在 owner-thread 且 State idle 时提供一致快照。

### 9.6 Production worker

- 只使用公开 C API。
- 组合 allocator cap、进程 CPU/内存边界、timeout、输出限制。
- 输出稳定 JSON 结果分类。
- 边界安装失败时 fail closed。
- 覆盖 success、instruction、timeout、cancel、allocator、process resource。

### 9.7 安全测试

- parser fuzz。
- bytecode verifier fuzz。
- stdlib numeric/path fuzz。
- ASan/UBSan/TSan 或 Rust 对应检查。
- unsafe hotspot Miri。
- cancellation/finalizer/state-close races。
- sandbox denied paths。

### 9.8 M4 完成门槛

- 与 C++ runtime public API 的结构和行为合同对齐。
- 所有预算、deadline、取消和 sandbox 拒绝路径有进程级测试。
- 1000 轮 runtime soak 无泄漏、无挂起、无重复 finalizer。
- worker 的成功和各类故障结果稳定。

## 10. M5：跨平台 SDK、性能与发布

### 10.1 跨平台

- 删除 workspace 默认强制 `x86_64-pc-windows-msvc`。
- stack/linker 参数放入 target-specific 配置。
- CI 至少覆盖：
  - Windows MSVC Debug/Release；
  - Linux stable/MSRV；
  - macOS；
  - 必要的 ARM64 lane。
- 规范化平台允许差异。

### 10.2 覆盖率

- 为以下组件建立硬阈值：
  - bytecode verifier；
  - C API；
  - GC phases；
  - opcode handlers；
  - parser/codegen；
  - sandbox denied paths。
- 阈值下降需要显式批准。

### 10.3 性能

- 建立与 C++ 同机配对 benchmark：
  - VM instruction throughput；
  - host→Lua；
  - Lua→host；
  - coroutine；
  - closure/upvalue；
  - GC pause/p99/max；
  - parser/codegen；
  - stdlib 热路径。
- 正确性完成前只记录，不用性能差异阻塞语义修复。
- 稳定后设置绝对 SLO 和相对回归预算。

### 10.4 SDK 与发布制品

- 安装头、静态库、共享库、Cargo artifacts。
- C/C++ 外部 consumer。
- 版本和 ABI metadata。
- SPDX SBOM。
- SHA-256 manifest。
- archive traversal/篡改检查。
- 解压后重新构建并执行 consumer。

### 10.5 发布治理

- release checklist。
- immutable RC tag。
- 同 SHA 全平台证据。
- nightly endurance。
- benchmark、coverage、fuzz、sanitizer。
- 目标环境 shadow/canary 和回滚演练。

### 10.6 M5 完成门槛

- 所有目标平台使用同一提交构建和验证。
- SDK consumer、SBOM、checksum 和归档验证通过。
- 性能无未批准的重大回归。
- release checklist 全部满足后才创建 RC。

## 11. CI 分层

### 11.1 PR 快速门

- fmt；
- clippy `--all-targets -D warnings`；
- Debug workspace tests；
- Release 核心测试；
- rustdoc `-D warnings`；
- basic/control-flow/functions/regressions；
- 4-case differential；
- 代表性 bytecode/VM trace；
- deviation/doc drift。

目标：在合理时间内阻止绝大多数语义和工程回归。

### 11.2 Nightly

- 全量 Lua fixture；
- official strict/slow；
- 全量 bytecode/VM trace；
- C API differential；
- sanitizer/Miri；
- fuzz 定时运行；
- coverage；
- runtime/module soak；
- benchmark 采样。

### 11.3 Release

- 全平台 Release。
- 123/123 C API。
- official suite。
- production worker 故障注入。
- 1000+ 轮 soak。
- SDK consumer。
- SBOM/checksum/archive。
- 绝对 SLO 和相对性能门。

## 12. 每个任务的完成定义

每个迁移任务必须包含：

```yaml
id: Mx.y-short-name
cpp_oracle:
  commit: 87c15e6
  files: []
rust_targets: []
lua_cases: []
unit_tests: []
differential:
  official_lua: required | not_applicable
  lua_cpp: required | not_applicable
unsafe_changes:
  count: 0
  proofs: []
acceptance: []
docs: []
known_deviations: []
```

PR 完成条件：

1. 有 C++ 参考位置和可观察行为说明。
2. 新增或更新失败测试。
3. Debug/Release 和适用的差分全部通过。
4. unsafe 块有完整 SAFETY 证明。
5. 没有新增未登记模拟或 unsupported 行为。
6. phase report、type mapping、glossary、deviation log 按需更新。
7. 工作树和生成 artifact 清理策略明确。

## 13. 推荐的首批 PR

PR-1 至 PR-3 的 M0 工作范围已在当前工作树完成；是否按三个独立提交/PR
拆分由合入策略决定。PR-4 和 PR-5 是当前 M1 主线，PR-6 仍严格依赖 PR-5。

### PR-1：质量门修复

范围：

- 修复 nextest 递归 alias。
- all-targets clippy。
- rustdoc `-D warnings`。
- Debug/Release test。
- 清理 104 条 lint 和 19 条文档 warning。

不得混入解释器语义修改。

### PR-2：进程 runner 与可观察 IO

范围：

- Lua fixture manifest。
- CLI 子进程 runner。
- 真实 stdin/stdout/stderr。
- `_VERSION`。
- 4-case differential。

目标是先获得可靠失败报告，不要求一次修完全部差异。

### PR-3：恢复 bytecode/VM trace parity

范围：

- 恢复比较脚本。
- 固定 C++ oracle。
- 建立结构化 artifact。
- 选择最小代表用例。

### PR-4：ByteString

范围：

- RFC。
- GcString/StringPool。
- loader/lexer/parser。
- string/io 边界。
- 任意字节和 UTF-8 回归测试。

这是大范围架构 PR，不与 GC 或 C API 混合。

### PR-5：Runtime 所有权和 deterministic shutdown

范围：

- Runtime owner。
- coroutine arena/handle。
- root inventory。
- 删除 `&'static mut` 辅助。
- collector/fixed object/State 关闭。
- 暂不启用自动 sweep。

### PR-6：真实 full GC

范围：

- full collect/sweep。
- 真实内存计数。
- collectgarbage/gcinfo。
- weak/finalizer/resurrection。
- write barrier。
- soak。

PR-5 完成前不得合并 PR-6。

### PR-7：源码兼容收敛

范围：

- 以差分报告为队列逐项修复。
- 优先处理调用、upvalue、coroutine、metamethod、错误。
- 每个问题保留最小回归 case。

## 14. 风险清单

| 风险 | 影响 | 缓解措施 |
|---|---|---|
| 在 root/所有权修复前启用 sweep | UAF、随机崩溃 | 强制 PR-5 前置，root inventory + soak |
| ByteString 改动面过大 | 编译器、stdlib、CLI 同时回归 | 独立 RFC/PR，双表示过渡必须有截止任务 |
| C++ `main` 持续变化 | 永远追不上目标 | 固定 SHA，只允许 baseline PR 升级 |
| 596 个内部测试造成错误安全感 | 近似实现被误判完成 | 以进程差分和 official suite 为完成门 |
| 模拟 GC 测试自证 | 测试绿但不回收 | 使用对象数、真实字节和析构 side effect |
| FFI panic/alias | 进程 abort 或 UB | catch boundary、opaque handle、Miri/sanitizer |
| 强制 Windows target | Linux/mac 无法验证 | 移除默认 target，建立平台矩阵 |
| 一次 PR 跨多个基础层 | 难审查、难定位回归 | 按首批 PR 边界拆分 |
| 性能优化提前 | 行为偏移、复杂度上升 | parity 完成前性能只记录不主导设计 |

## 15. 进度报告方式

不再使用“文件已存在”或“函数已注册”作为完成指标。每周状态应按以下格式报告：

| 领域 | 通过合同 | 总合同 | 未解释差异 | P0 blocker | 趋势 |
|---|---:|---:|---:|---:|---|
| CLI behavior |  |  |  |  |  |
| Bytecode |  |  |  |  |  |
| VM trace |  |  |  |  |  |
| Official Lua |  |  |  |  |  |
| GC/lifecycle |  |  |  |  |  |
| Standard library |  |  |  |  |  |
| C API |  |  |  |  |  |
| Production runtime |  |  |  |  |  |

每个阶段只在对应完成门槛全部满足后标记完成。若仍有 expected failure，必须列出 deviation ID、
维护者、原因和计划移除版本。

## 16. 下一步

M0 已在本地完成，M1 基础层已经完成 coroutine activation trampoline、compiler
Proto→Function、library/package、IO construction、VM/app/result publication
切片、production string canonicalization/scoped Eq/Hash 合同、唯一
Heap/service owner、weak/finalizer/resurrection、Lua-visible full STW、production
mutation barrier、Runtime-owned incremental GC、managed allocator 计账、
allocation-triggered automatic GC、五点 allocator failure injection、main-state
StateArena ownership、显式 IO resource drain、本地 Miri、Windows ASan、1000 轮
组合生命周期矩阵、1000-resource drain 与 arbitrary callback suspension。
Linux 验证按当前环境策略延期；M1.7 通用非字符串 scoped access audit 已在本地闭环，
完整 M0/M1 gate、raw-byte 双 oracle、fresh non-official 与代表性 bytecode parity
artifacts 也已按第 6 项刷新。M2 P0-B 已补齐 byte-safe constant/local 证据，并依次修复
开放式表字段 `NEWTABLE` 容量、lvalue-before-RHS assignment freezing、relocatable
arithmetic result、for-loop iteration-scope `CLOSE`、boolean-only short-circuit jump list、
root-scope `CLOSE` 与 lineInfo；103 行 closure pipeline 已达到完整 structured parity。
随后又对齐 helper table/concat/line scheduling、member-call temporary frame、local call
multi-return frame、nil RK constant 与 nested-call line guard。5-case integration corpus 和显式
131-case full-valid structured corpus 均已零差异，两批语料合计覆盖固定 C++ 合同中的 36/36
frontend-produced opcode；`NOT/TESTSET` 明确 intentionally-not-produced。M2.7 已接入
真实 JSONL Snapshot/Diff、固定 C++ evidence adapter 与双 12-case Basic 硬门，并完成
Complete 核心 lifecycle 双模式 4-case；下一步关闭 callback `stackTop` 与固定 C++ adapter
state-generation/GC-retirement oracle 缺口：

1. 已完成 LightUserdata 拆型、`ObjectId + collector live table` 与字符串
   identity/scoped access 合同；唯一 HeapId owner、fixed strings、
   pending-finalizer root seed 和 Runtime root-tracer 接线也已闭环。
2. top-level/active/debug Proto 已全部改为受管 `GcRef<Proto>`；
   RuntimeId/StateHandle issuance 与 slot generation exhaustion 也已
   fail-closed；scoped mailbox、`VmExit::NativeRequest`、deferred C frame
   与 activation stack 已把生产 resume/wrap 接入 Runtime 调度，并保持每
   turn 单 state 借用和 release-before-switch。open Upvalue 已改为
   `StateHandle + stack index`，远端读写、owner-state root 入队和
   close-before-generation-advance 均已接入；`debug.getupvalue/setupvalue`
   已通过 sealed Runtime-native mailbox 调度远端 owner turn，`pcall` 使用
   protected 回包，foreign/stale owner 转为 fail-closed 错误响应。
3. lexical temporary-object registry、HRTB `PublicationTxn/Rooted`、panic/nested
   cleanup、mark seed 与 `TEMPORARY_STATE_ROOTS/PendingState` 已完成；
   coroutine create/wrap 的 State→Thread→stack、compiler 内部 string/child
   Proto builder、顶层 Proto→Function 以及 CLI/bytecode/base/package loader
   handoff、全部标准库注册、package/loaded/preload/nested-module graph 以及
   IO file/lines、VM temporaries/open-Upvalue、Runtime roots/errors、CLI
   argument buffers 与同步 call/top-level results 均已事务化；目标生产路径
   直接 `gc.create` 为 0。生产字符串构造也已强制经过 StringPool，
   `Value::String` Eq/Hash 使用 canonical identity，内容读取经 collector/state
   作用域验证；pending finalizers 与 fixed strings 已进入 canonical roots。
4. 已建立唯一 `Heap` owner，移除 LuaState GC/StringPool backpointer，并以
HeapId/54-path gate 拒绝 production standalone collector/pool 错配。full STW
   在 owner-thread Runtime safe point 消费 canonical tracer，按
   mark→finalizer prepare/resurrection propagation→weak reconcile→state
   prepass→sweep→protected callback 执行，并拒绝任何 trace gap/foreign edge。
5. weak key/value/kv、protected finalizer、nested collect 非递归 drain、异常
   隔离、exactly-once、resurrection/再次不可达、close drain、Lua-visible
   `collectgarbage("collect")` 和五阶段 `step` 已闭合；`count`/`gcinfo`
   使用 collector 计账。8-family production mutation inventory、checked
   `with_mut` barrier、active-allocation publication、debt/pause/stepmul 和
   Runtime StateHandle/object 双队列已有静态门与回归。
6. managed owner/publication-root 树稳定后已重跑完整 M0/M1 gate、raw-byte 双 oracle、fresh
   101-case non-official 与代表性 bytecode parity，并更新精确 artifact/SHA；本地完整
   quality gate 已实际执行 `cargo audit`，远程 CI 首次运行仍必须再次执行。
7. M2 backlog 中的 12 个 process status debt 与完整 VM trace profile 继续作为显式输入。
   固定 C++ `Proto` 已由 checkout 外只读 sidecar 输出 constant/local byte/bits envelope，并与
   pristine printer 逐 Proto exact-path 合并；原两个 bytecode case、closure pipeline、
   5-case integration corpus 与显式 131-case full-valid corpus 均为零差异，且后两批覆盖
   36/36 frontend-produced opcode。真实 Snapshot/Basic 与 Diff/Basic 代表集均为 12/12；
   Complete 核心 Snapshot/Diff 双 4-case 覆盖全部八类事件并零差异；下一入口是 callback
   `stackTop` 与 C++ adapter state-generation/GC-retirement oracle 缺口，且不得用 normalization
   静默消除任一缺口。

在上述 M1 硬门全部关闭前，不开始正式 C API、binary chunk、动态模块，也不
把 core 单元测试中的 `sweep` 接到 live VM。

### 16.1 当前续接检查点（2026-07-29）

本轮收口时已经完成并验证：

- P1 `ObjectId + collector live table` provenance、LightUserdata 拆型、持久
  root/weak/finalizer/external 队列身份校验；
- managed active/debug Proto、frame return/error/unwind 清理与 canonical
  roots；运行时 `*const/*mut Proto` 静态命中为 0，collector tag dispatch
  两处强转除外；
- deterministic Runtime shutdown 基础和 1,000 轮归零；
- 临时对象/状态根：`PublicationTxn/Rooted`、compile-fail 防逃逸、nested/panic
  cleanup、mark seed、1,000 scope zero，以及 exact-id `PendingState` 的
  rollback/commit、独立 state root seed 和 coroutine create/wrap 直接压栈发布；
  compiler 的 interned string/child Proto/top Proto/Function 也已在同一事务
  中构造，并由 explicit root 或活动 Lua stack 接管；标准库 Table/key/
  C-or-runtime-native Function 注册、package 对象图以及 IO file/lines 图也已由
  traced Table/stack 接管；VM temporaries/open-Upvalue、Runtime roots/errors、
  CLI argument buffers 与同步 call/top-level results 也已迁移；
- bytecode schema v2 和首个指令形状切片。证据位于
  `target/compatibility/bytecode-schema-v2-original-two/report.json` 与
  `target/compatibility/bytecode-closure-schema-v2/report.json`（target
  artifact 不提交）。
- StateHandle identity/generation 前置切片：并发 checked RuntimeId、
  non-Clone issuer、raw 构造 compile-fail、MAX-generation retirement、
  free-list/count preflight 与真实 stale/foreign trace 均有回归。
- Runtime coroutine activation trampoline：sealed runtime-native
  resume/wrap、scoped mailbox、独立 VM exit、deferred C frame、activation
  stack、generic-for continuation 与 canonical activation-buffer root seed
  已接入生产 app/stdlib 入口；caller borrow 在 target resolve 前释放。
- open-Upvalue owner：`LuaState` 维护按栈索引降序且去重的 checked
  `GcRef<Upvalue>` 集合，Upvalue 仅保存 `StateHandle + stack index`；
  suspended coroutine closure 的远端 GET/SET 经 Runtime owner turn
  完成，root tracer 可从 reachable Upvalue 入队 owner state，arena drain
  会在 handle generation advance/retirement 前关闭节点。Normal 祖先重放
  上下文也能跨 Upvalue turn 保持。
- 固定 C++ `Normal` 祖先重入 characterization 已独立锁定：
  C++ stdout SHA-256 为
  `bad37c42fcfd369f22fdc9d9ec8d1ce46caaa2e8fa755fe03a41a6e91b2591d2`，
  stock 为
  `0488432cb01117da75f229ab0f43bd1c1ea174853ebde5f1ab62853265f805f6`；
  Rust process regression 已逐字节匹配固定 C++ 输出；该 manifest 仍是
  non-gating characterization、无 approved deviation，因为 stock Lua 行为
  仍与项目目标不同。
- 最新验证：fmt/check、Debug/Release 各 815 个 workspace tests、all-targets Clippy、
  warning-free rustdoc、24/24 root inventory、17-path string contract、
  8/8 mutation inventory、54-path heap contract、差分比较器与 parity runner
  自测全部通过；
  fixture manifest 校验现为 132 项；两个 coroutine oracle
  各重复 3 次通过精确字节校验。characterization checker 已同时通过
  Windows PowerShell 5.1 与 PowerShell 7，并将实际执行文件的 SHA-256
  绑定到现有 C++/Lua 5.1.5 provenance build reports。M1 smoke 汇总为
  `checksPassed=true`、`hardFailures=[]`；因显式 smoke/audit skip，按策略
  保持 `foundationPassed=false`，不代表 M1 完成。当前 raw-byte differential
  已使用 `target/oracles/lua_cpp-source@87c15e6` 完整通过，并未跳过。
- 本轮 coroutine characterization 复验在执行行为比较前 fail-closed：
  工作区 C++ checkout 为 `14f5250`，与锁定 oracle `87c15e6` 不同；既有
  provenance artifact 保留，但不得把当前 checkout 的结果记为新 oracle，
  也不得在本任务中静默更新 manifest。

下次不要从头审计，按以下顺序继续：

1. 已完成 StateHandle 前置切片：checked/concurrent RuntimeId allocator、
   non-Clone issuer namespace、raw 安全构造 compile-fail、generation
   `u64::MAX` 最后一代与永久 slot retirement、free-list/count shutdown
   preflight；不存在 `fetch_add`/`wrapping_add` 回绕路径。
2. 已完成 coroutine trampoline 生产切片：Runtime 每 turn 单借用、
   release-before-switch、scoped mailbox、`VmExit::NativeRequest`、
   deferred C frame 与 activation stack 已接入 app/stdlib；固定 C++ 允许
   `Normal` 祖先重入及 continuation 二次执行的精确输出已锁定。
3. 已完成 open Upvalue 的
   `Open { owner: StateHandle, index }`、非 intrusive state-owned 集合、
   reachable owner-state 入队、远端 Runtime turn 和
   close-before-generation-advance；debug get/setup 已复用 boxed
   Runtime-native owner turn 和 protected 回包；arbitrary-Lua `pcall/xpcall`
   activation、深链与 fault matrix 也已完成。其他同步 callback helper
   suspension 是下一入口。
4. 已完成 `TEMPORARY_STATE_ROOTS/PendingState`：未发布状态可独立追踪，
   插入/绑定/压栈故障会 exact-id 回滚，`u64::MAX` 仅退休一次；create/wrap
   已用 typed Thread/Upvalue/Function publication 接入生产路径。
5. 已完成当前第一开发项：`CodeGenerator` 的 sealed publication allocator
   在 builder 全程保护 interned strings 与 child Proto，顶层 Proto→Function/
   environment 通过 typed API 构造；CLI、bytecode tool、base load/dofile 与
   package file loader 已在 explicit root 或活动 Lua stack 接管后才释放临时根。
   第二开发项 library/package 也已完成：共享注册层保护 destination Table、
   canonical key、C/Runtime-native Function 与 environment；catalog library
   Table、package/loaded/preload、嵌套 module path、metadata/metatable 与错误
   string 仅在 traced Table/stack 接管后释放。第三开发项 IO construction graph
   也已完成：file Userdata/metatable/method 与 lines
   Function→environment→file 图均事务发布，字段/result/error string 走 typed
   Table/stack publication，`io.rs` 生产 `gc.create` 为 0。第四开发项
   VM/app/results 也已完成：callee result `Vec<Value>` 逃逸被同步 callback
   替代，目标生产文件直接 `gc.create` 为 0。第五开发项 production string
   canonicalization/scoped Eq/Hash 也已完成：生产构造强制池化，身份
   Eq/Hash、作用域 byte access 与静态 inventory/gate 均已落地；继续维持
   allocation-triggered collection 禁用。
6. 已完成唯一 Heap/service owner、fixed/pending-finalizer roots、Runtime
   tracer、全图 STW、weak/finalizer/resurrection、Lua-visible
   `collectgarbage("collect")`、8-family mutation barrier 与 explicit
   incremental phase/debt/step、allocator live/peak/total accounting 与
   allocation-triggered Runtime checkpoint；下一步补 Miri/ASan 和分配失败矩阵。
7. M2 并行下一步是为固定 C++ bytecode printer 的 local-name 证据建立受审计
   方案，并从 closure artifact 的第一个 Proto/PC 差异开始修，不能直接处理
   截断后的 500 条列表。

推荐恢复后先运行：

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets --quiet
cargo clippy --workspace --all-targets -- -D warnings
pwsh -NoProfile -File tools/check_string_contract.ps1
pwsh -NoProfile -File tools/check_heap_contract.ps1
pwsh -NoProfile -File tools/check_gc_root_inventory.ps1
pwsh -NoProfile -File tools/check_coroutine_normal_ancestor_characterization.ps1
pwsh -NoProfile -File tools/run_lua51_differential.ps1 -ComparatorSelfTestOnly
```

### 16.2 新会话恢复执行单

本节是新会话的权威接力入口。开始工作时先读本节，再按需回看 16.1 和 M1.7–M1.13；
不要重新推断已经关闭的 compiler、library/package、coroutine 或 open-Upvalue 任务。

#### 16.2.1 当前实现快照

- 当前工作树基于 `f7e8380` 并包含累计 M1 ownership/callback/main-state/resource-drain
  工作树切片；
  交接时必须先运行 `git status --short`，保留用户与未提交实现改动，不得用
  reset/checkout 清除。
- 当前仍是 M1 `active`。weak/finalizer/resurrection、Lua-visible full STW、
  production write barrier、explicit incremental phase/debt/step、managed
  allocator accounting、allocation-triggered automatic collection、五点
  allocator failure injection、本地 Miri、Windows ASan、1000 轮组合 lifecycle、
  1000-resource drain、main-state arena ownership 与 arbitrary callback matrix
  已完成；M1.7 通用 non-string scoped read 已建立 API，`lua_stdlib/src` 与
  `lua_vm/src/execute.rs` 的生产直接 managed-object access 已清零，VmContext/StringPool alias
  边界也已通过全量 stdlib Miri 和 VM 定向 Miri。完整 M0/M1 gate、双 oracle 与 fresh
  artifact 已刷新；固定 C++ byte-safe constant/local 证据、首个 nested `NEWTABLE`、assignment/register
  scheduling、for-loop iteration-scope `CLOSE`、boolean jump list、root-scope `CLOSE`、
  call-frame scheduling、RK constant 与 nested-call lineInfo 也已对齐；closure pipeline、
  5-case integration corpus 和显式 131-case full-valid corpus 均为零差异，后两批达到固定
  C++ 合同的 36/36 frontend-produced opcode 覆盖。下一入口是扩展更大有效 corpus。
- 已关闭与待办边界如下：

| publication/owner 子项 | 状态 | 当前证据或下一入口 |
|---|---|---|
| temporary object/state roots、managed Proto、coroutine activation、open Upvalue owner | `completed-slice` | 见 16.1；对应 root seed、rollback、远端 owner turn 和 shutdown 回归已通过 |
| debug/protected open-Upvalue scheduling | `completed-local-slice` | `debug.getupvalue/setupvalue` 通过 boxed Runtime-native request 执行远端 owner turn；direct/`pcall`、foreign/stale、close-before-invalidate、shutdown 与 Windows ASan 定向回归已通过 |
| compiler Proto→Function publication | `completed-slice` | compiler builder 与 CLI/bytecode/base/package loader handoff 已事务化 |
| library registration 与 package graph | `completed-slice` | `registration.rs` 共享层；catalog、package/loaded/preload、nested module、metadata/metatable 已事务化 |
| IO construction/publication graph | `completed-slice` | file Userdata/metatable/method 与 lines Function/environment/file 图已事务化；生产 `io.rs` 直接 `gc.create` 为 0，故障清理、mark-only、全回收与行为回归已通过 |
| Runtime resource drain | `completed-local` | graph-owned IO Userdata 在发布前预留 weak `ObjectId + hook` 注册；close 按 finalizer→resource→coroutine/main state→heap 顺序执行，失败刷盘、panic/error、preflight retry、allocator rollback 与 1000-resource 回归通过；package 当前无 native handle |
| 通用 non-string scoped access | `completed-local` | `LuaState::with_gc_ref<T>` 与显式 collector scoped API 提供 address/ObjectId/type 验证和 HRTB callback；stdlib 与 VM dispatch 生产直接 `as_ref/as_mut` 均清零并有全源静态守卫；`with_vm_context_parts` 已删除，显式 collector context fail-closed 隐藏动态 collector、保留 StringPool，普通 C callback 通过窄嵌套 context 恢复动态服务；stdlib Miri 23/23 与两项 VM 边界 Miri 通过 |
| VM/app/result publication | `completed-slice` | VM Table/closure/string/open-Upvalue、Runtime roots/errors、CLI args 与同步 call/top-level result publication 已事务化；目标生产 direct-create 为 0 |
| production string canonicalization/scoped Eq/Hash | `completed-local` | 生产构造强制 StringPool canonical identity；`Value::String` Eq/Hash 不解引用；内容比较/排序/展示走 collector/state-scoped bytes；17-path inventory、静态门与 duplicate/foreign/stale/NUL/high-byte/address-reuse 回归已落地 |
| 唯一 Heap/service owner | `completed-slice` | `RuntimeStorage` 唯一持有 Heap/StateArena/activation service；HeapId 绑定 collector/accounting 与 canonical StringPool；LuaState 无 service backpointer；fixed/pending-finalizer roots 与 54-path heap gate 已落地 |
| Runtime full collection | `completed-local` | direct crate-private 与 Runtime-native safe-point STW 均强制消费 canonical tracer，在 generation 失效前预关闭不可达 state/open Upvalue，并真实 sweep 全图、更新 object/accounted-byte 计账；所有 gap/foreign edge fail-closed |
| Weak/finalizer/resurrection + Lua-visible collection | `completed-local` | weak key/value/kv、pending/finalized userdata、protected `__gc`、nested collect、异常隔离/保留队列、exactly-once、resurrection/再次不可达与 close drain 回归已通过；full 与 incremental 完成阶段都使用同一 protected finalizer delivery |
| Write barrier + incremental GC | `completed-local` | 8-family mutation inventory、checked post-write barrier、active-allocation initial graph、五阶段、debt/pause/stepmul、真实 work unit、Thread→StateHandle 双队列与 weak/finalizer 多周期回归已落地 |
| Allocator accounting + automatic GC | `completed-local` | allocator live/peak/total、对象动态大小对账、阈值 checkpoint、自动推进门与 11/11 allocator contract 已落地 |
| Allocator failure + sanitizer durability | `completed-local-slice` | GC object/StringPool key/publication root/StateArena slot/resource registry one-shot failure 均在 owner mutation 前触发并可重试；3 个 core + 2 个 Runtime tests 和 1000 轮 pending-state soak 在本地 Miri 通过；Windows ASan 通过原 5 个 cases 及 1000 轮 coroutine/weak/finalizer/closure-upvalue 组合矩阵。新增 registry case 已通过本地 Debug/Release，Linux/补充 sanitizer 验证延期 |

当前 Debug/Release 各 886 个 workspace tests、fmt、all-targets
check/Clippy、warning-free rustdoc、24/24 root inventory、8/8 mutation
inventory、11/11 allocator、17-path string contract 与 54-path heap contract
均通过。全量 stdlib Miri 23/23、VM 普通 C callback 重入和显式 collector
fail-closed 两项定向 Miri 均通过；本次新增的 debug normal/protected 与
foreign/stale 两条路径也已通过 Windows ASan。当前
完整 M1 foundation gate 为 `checksPassed=true`、`foundationPassed=true`、
`m1Complete=false`、`hardFailures=[]`；quality/differential/audit 均未跳过。
`foundationPassed=true` 只表示当前 foundation gate 全通过，不能改写为 M1
整体完成。

#### 16.2.2 已完成主任务：IO construction graph

目标：关闭 `io.rs` 在“分配对象”到“被活动 Lua stack、traced Table、Function
environment 或显式 root 接管”之间的窗口。核心不变量是：任何新 GC 对象在
`PublicationTxn` 临时根释放前，必须已经形成可由 canonical tracer 到达的 durable
edge；不能先返回裸 `GcRef` 再由调用方择机发布。

先锁定以下两张生产对象图：

```text
global Table
  -> io Table
     -> __stdin/__stdout/__stderr
        -> Userdata
           -> file-state/metatable Table
              -> __index (self)
              -> method-name GcString -> C Function
              -> state-field-name GcString -> field Value

active Lua stack
  -> lines iterator Function
     -> environment Table
        -> __file -> file Userdata
        -> __auto_close/__dead -> scalar Value
```

迁移前窗口都在 `crates/lua_stdlib/src/io.rs`，按函数名追踪而不是依赖易漂移的
行号：

1. `reg_unpublished_table` 在本地 `Table` 尚未受管时直接分配 method key 和
   C Function；
2. `create_file` 先用直接分配填充 file-state Table，再分配 `__index`、state
   Table 和 Userdata，并把裸 Userdata `GcRef` 返回给稍后才压栈/入表的调用方；
3. `push_lines_iterator` 分别分配 environment Table 和 iterator Function，
   Function→environment→file 以及 stack→Function 的接管不在同一事务；
4. `set_*_field`、`set_table_*` 与 `push_lua_*` helper 仍含生产
   `gc.create`。它们是 IO 第二小切片，必须在宣布“IO publication 全部完成”前
   一并清零。

#### 16.2.3 已执行顺序

1. **先写失败测试和对象图断言。**
   为 file graph 与 lines iterator graph 分别增加 mark-only reachability、
   transaction-drop、panic/failure cleanup 和最终全回收测试。至少覆盖在 method
   注册中途、`__index` self-edge 前后、Userdata metatable 接线前后、iterator
   environment 接线前后的故障点。
2. **补最小 typed publication API。**
   优先复用 `lua_core::gc::PublicationTxn` 和
   `lua_stdlib::registration`。只在缺失时增加 Userdata 分配/metatable、
   `Table -> Userdata/Value`、`Function -> environment`、stack publication
   helper；所有 helper 必须校验 collector provenance，并保持 HRTB
   `Rooted` 不可逃逸。
3. **IO-1：迁移构造器。**
   删除 `reg_unpublished_table`；把 file-state Table、keys、method Functions、
   self `__index`、Userdata 和 metatable edge 放进同一 transaction。重构
   `create_file`/`open_file_handle` 的合同，使成功路径在事务结束前直接发布到
   明确 destination。若文件系统打开逻辑需要提前返回，先返回非 GC payload，
   再由 destination-aware 构造器分配 GC 图。
4. **IO-1：迁移 iterator。**
   在同一 transaction 中建立
   `Function -> environment -> file Userdata`，随后把 Function 压入活动栈，
   再释放临时根。auto-close 与 error 路径也必须保留准确的接管/清理语义。
5. **IO-2：清理 IO 内其余生产分配。**
   把 field mutation 的 key/value strings 和结果/error strings 改为 typed
   table/stack publication；最终 `io.rs` 的生产代码不得再直接调用
   `gc.create`。若测试自身需要 direct create，必须限制在 `#[cfg(test)]`
   并由静态 gate 明确排除，不能让生产例外混入 allowlist。
6. **接入静态门和更新证据。**
   在 `tools/m1_foundation_gate.ps1` 增加 IO direct-publication 检查，更新
   `tests/compatibility/gc_root_inventory.json`、ownership RFC、phase report、
   deviation log 和本节测试计数。不要只改 `plan.md` 宣布完成。

#### 16.2.4 IO 完成证据

以下条件已全部成立，IO 行已改为 `completed-slice`，下一入口为 VM/app/results：

- `reg_unpublished_table` 已删除，`io.rs` 生产路径直接 `gc.create` 命中为 0；
- file Userdata、metatable/state Table、method Functions/keys、iterator
  Function/environment/file 的每条强边均由 typed API 建立；
- success、early return、foreign/stale collector、panic/failure injection 后
  temporary root 数都精确归零，且 foreign edge 在 mutation 前被拒绝；
- 从 global Table 或活动 Lua stack 开始 mark-only tracing 时，新图全部可达、
  无 rejected mark edge；移除 durable roots 后对象可完整回收；
- stdin/stdout/stderr、`io.open/tmpfile/input/output/lines`、file methods、
  auto-close 与 arbitrary-byte string 现有行为不回归；
- workspace 测试数为 766，且新增 8 项 IO 测试全部进入默认 test run；
- fmt、workspace all-targets check/test、Clippy `-D warnings`、warning-free
  rustdoc、24/24 root inventory、M1 smoke 全部通过，`git diff --check` 为 0。

建议使用独立验证目录，避免误用旧构建产物：

```powershell
cargo fmt --all -- --check
cargo check --target-dir target/verify-io-publication --workspace --all-targets
cargo test --target-dir target/verify-io-publication --workspace --all-targets --quiet
cargo clippy --target-dir target/verify-io-publication --workspace --all-targets -- -D warnings
$env:RUSTDOCFLAGS = "-D warnings"
cargo doc --target-dir target/verify-io-publication --workspace --no-deps
pwsh -NoProfile -File tools/check_gc_root_inventory.ps1
pwsh -NoProfile -File tools/m1_foundation_gate.ps1 `
  -Smoke -SkipQualityGate -SkipDifferential -SkipAudit `
  -ResultPath target/compatibility/m1-foundation-io-publication-smoke.json
git diff --check
```

#### 16.2.5 本切片保持的边界

- 不启用 allocation-triggered collection 或把单元测试中的 sweep 接入 live VM；
- 不在 IO 迁移中顺带实现唯一 Heap owner、write barrier、weak table、finalizer、
  resurrection、真实内存计数或 `collectgarbage`；
- 不把 IO 行为近似修复、M2 的 88 个进程差异、C API、binary chunk 或动态模块
  混入同一改动；
- 不因 mark-only reachability 测试通过而宣称 destructive GC 安全。

IO 收口后的固定顺序已经执行到 weak/finalizer/resurrection、真实
`collectgarbage("collect")`、production mutation inventory/write barrier
与 explicit incremental phase/debt/step、allocator accounting、automatic GC、
fault injection、本地 Miri、Windows ASan 和组合 lifecycle matrix。Linux
验证按当前策略延期；debug/protected Runtime-native open-Upvalue 矩阵也已
完成，下一步补 M1.5 深链与 broader coroutine fault 矩阵。

### 16.3 已完成主任务：VM/app/result publication

#### 16.3.1 完成边界

- VM `NEWTABLE`、`CLOSURE`、concat/debug/error string、legacy vararg Table
  与 open Upvalue 在 register/state owner 接管前由 `PublicationTxn` 保护；
- Runtime global/registry Table 由 explicit roots 接管，coroutine error 在
  `LuaState::last_error` 接管后才释放临时根；
- CLI `arg` Table 以 typed numeric/string edge 构造，script varargs 以 explicit
  string roots 跨过 `Runtime` 调用边界，执行结束后 exact root cleanup；
- `Runtime::execute_proto_with_args` 在 main stack 清空前同步消费结果；
  `call_value_with_results` 在恢复 caller stack 后仍以 exact-id temporary roots
  保护 collectable result slice；两种 callback 都只能返回 `()`，从 API
  形状上禁止 collectable result 作为 Rust 返回值逃逸；
- base/coroutine/debug/math/os/string/table 的结果 Table、Function、string 与
  proxy Userdata 全部发布到活动 stack/Table；目标 production prefix 直接
  `gc.create` 命中为 0，旧 `call_value` API 命中为 0。

#### 16.3.2 新增证据

- `result_slice_is_rooted_for_callback_and_released_after_consumption`：
  callback 内 mark-only 可见所有 collectable result roots，返回后 registry 为 0；
- `result_slice_foreign_failure_and_panic_cleanup_exact_roots`：
  foreign edge 不执行 callback，partial validation 与 panic 都 exact-id 清理；
- `protected_call_results_are_published_before_stack_window_restores`：
  callee stack 已恢复时 result 仍受临时根保护，并可原子发布到 caller stack；
- `script_argument_table_and_varargs_survive_runtime_publication_handoff`：
  进程级验证 `arg[0..2]` 与 chunk `...` 同时跨 Runtime handoff 保持正确；
- `xpcall_handler_failure_keeps_false_and_one_published_error_result`：
  error handler 再失败时临时错误栈根被移除，公开结果仍严格为
  `false, handler_error`；
- workspace 默认测试数由 766 增至 771。

#### 16.3.3 已完成主任务：production string canonicalization/scoped Eq/Hash

1. `Value::String` 的 `Eq/Hash` 只比较/散列不可复用的 `GcRef` identity，
   不再从 safe trait 实现中解引用对象；canonical StringPool 保证正常生产值的
   Lua byte-equality 与 identity 一致。
2. compiler 已删除无 StringPool 构造入口；VM、stdlib、app、参数/错误/字段/
   metamethod/name 路径统一通过 publication pool 构造字符串，raw
   `GcString::from_bytes` 仅保留为 `lua_core` 内部构造能力。
3. 内容比较、排序、数字转换、长度、输出和诊断通过
   `GarbageCollector::with_string_bytes`、`LuaState::with_string_bytes` 或复制型
   作用域 API 完成 foreign/stale/type 校验；Table/metamethod/library/name lookup
   使用同一池中的 canonical identity。
4. [`string_access_inventory.json`](tests/compatibility/string_access_inventory.json)
   记录 17 条生产边界；`tools/check_string_contract.ps1` 检查 raw 构造、
   unscoped dereference、compiler fallback 和 `Value` trait 形状，并已接入
   `m1_foundation_gate.ps1`。
5. 回归覆盖重复 bytes、foreign/stale string、embedded NUL/high-byte Table
   key、safe Eq/Hash、ObjectId 地址复用与作用域拒绝；allocation-triggered
   collection 仍保持禁用，因为 owner/root/tracer 合同尚未闭合。
6. Debug 与 Release 各 772 项 workspace tests、fmt/check、all-targets Clippy
   `-D warnings`、warning-free rustdoc、17-path string contract、24/24 root
   inventory 与两条 oracle lane 的 raw-byte differential 均通过。M1 smoke
   为 `checksPassed=true`、`hardFailures=[]`；本机未安装 `cargo-audit`，因此
   audit 被显式跳过且 `foundationPassed=false`。

#### 16.3.4 已完成主任务：唯一 Heap/service owner

1. `lua_core::Heap` 以进程级不复用 `HeapId` 共同持有
   `GarbageCollector`、现有 object/estimated-byte accounting 与 canonical
   `StringPool`；跨 Heap collector/pool 组合在读取对象前 fail-closed。
2. pinned `RuntimeStorage` 唯一持有 Heap、StateArena 与
   `NativeActivationStack`。app、bytecode tool、compiler、VM 和 stdlib 的
   production prefix 不再构造 standalone `GarbageCollector::new` 或
   `StringPool::new`。
3. `LuaState` 的 GC/StringPool raw backpointer 已删除。Runtime 每次只在独占
   state turn 的动态范围安装 `ActiveVmContext`；nested scope、panic unwind、
   inactive state 和跨 Heap service 错配均有回归。
4. Runtime 初始化 canonical metamethod、emergency error 与 `_ENV` fixed
   strings；`begin_mark_only` identity-check 并 seed pending finalizers。
   `Runtime::trace_roots_mark_only` 对 fixed、pending、temporary 与 activation
   roots 使用 inventory 对齐标签。
5. `Heap::destroy_all/Drop` 在 StringPool 存活时释放普通与 fixed 对象；
   standalone collector Drop 也按类型释放全部 registered allocation，不再
   仅 unlink 泄漏。自定义 Lua allocator callback 与 allocator live/peak
   仍由 M1.13 跟踪，不在本切片夸大完成。
6. 新增 10 项回归，workspace tests 由 772 增至 782；24/24 root inventory
   当前为 21 partial、3 implemented、0 missing、0 unsafe。新增 52-path
   `check_heap_contract.ps1` 并接入 M1 foundation gate；fmt、all-targets
   check/test/Clippy、warning-free rustdoc 与 smoke 均通过，smoke 的
   `hardFailures=[]`。allocation-triggered collection 继续禁用。

#### 16.3.5 已完成主任务：Runtime-only stop-the-world full collection

1. 新增 crate-private `Runtime::collect_full_stw`，只允许 owner thread、
   `Running` phase、零 active execution 进入；每次 destructive cycle 先消费
   canonical Runtime tracer，并在任何 state/root gap、foreign/rejected edge 或
   未耗尽 mark work 时拒绝 sweep。
2. `StateArena::sweep_unreachable_owned` 在 mutation 前完成 arena/free-list
   preflight；不可达 coroutine state 先关闭 open Upvalue，再推进/退休
   `StateHandle` generation 并释放 state，最后才进入 object sweep。
3. Heap 以同一 collector/canonical StringPool 执行真实 sweep，报告 object、
   accounted-byte、interned-string 与 coroutine-state 前后值。回归覆盖两轮
   reachable/unreachable 回收、typed Userdata destructor exactly-once、stale
   handle、Upvalue close order、root gap、cross-collector edge、weak/finalizer
   与 phase/active-execution gate。
4. active weak table、pending/new finalizer 会显式返回 unsupported，不运行回调、
   不执行 object sweep；public `collectgarbage` 与 allocation-triggered/
   incremental collection 继续禁用。workspace 新增 8 项测试，由 782 增至
   790；heap contract 由 52 增至 53 个 production paths。
5. Debug/Release workspace tests、fmt、all-targets check/Clippy、warning-free
   rustdoc、24/24 root inventory、17-path string contract、53-path heap contract
   与 M1 smoke 均通过；smoke 为 `checksPassed=true`、`hardFailures=[]`，因显式
   skip audit/differential 且 M1 未完成，`foundationPassed=false`。

#### 16.3.6 已完成主任务：weak/finalizer/resurrection 与 Lua-visible full collection

1. full STW atomic 已固定为 mark→finalizer candidate prepare→resurrection
   graph 再传播→weak key/value/kv reconciliation→state prepass→sweep；
   pending finalizer 在 callback 前保持 canonical root，弱值在 callback 前删除，
   弱键在 pending/resurrected 阶段保留，并在后续再次不可达时清理。
2. Runtime activation service 新增 GC maintenance frame，在释放当前
   StateArena turn borrow 后运行 destructive safe point；callback 以独立
   activation 执行并保持 deferred caller snapshot、GC frame 与当前 userdata
   可追踪。nested `collectgarbage("collect")` 可重入 collection，但不会递归
   drain 外层 finalizer queue。
3. `__gc` delivery 已覆盖 exactly-once、resurrection→再次不可达、普通错误向
   Lua 传播、`pcall` 隔离，以及发生错误时保留其余队列供下一次 collection。
   Runtime close 会尝试全部剩余和 callback 新分配的 finalizable userdata，
   隔离错误并继续 drain；finalizer 内跨 state resume/open-Upvalue 与 close-time
   Runtime-native 重入当前明确 fail-closed。
4. base `collectgarbage("collect")` 已调用真实 full STW，`count`/`gcinfo`
   读取 collector 实际 accounted bytes；compatibility `step` 倒计时完成时也
   触发真实 full STW，但 phase/debt/work-unit 语义仍由 M1.10 实现。
5. 新增 13 项 workspace 回归，覆盖 weak v/k/kv、pending/finalized userdata、
   finalizer Thread→StateHandle 图、protected error、queue retention、nested
   main-Lua continuation、reentrant collect、resurrection/再次死亡和 close
   drain；workspace tests 由 790 增至 803。
6. fmt、all-targets check/Clippy、Debug/Release 803 项 workspace tests、
   warning-free rustdoc、17-path string contract、53-path heap contract 与
   24/24 root inventory 均通过；固定 `lua_cpp@87c15e6` 和 Lua 5.1.5 两条
   differential lane 均为 4/4。M1 smoke 为 `checksPassed=true`、
   `hardFailures=[]`；因显式 skip quality/differential/audit 且 M1 尚未完成，
   `foundationPassed=false`。

#### 16.3.7 已完成主任务：write barrier mutation inventory 与 incremental GC

1. 新增 8-family `gc_mutation_inventory.json` 与 fail-closed
   `check_gc_mutation_contract.ps1`，覆盖 Table、Function、Proto、Upvalue、
   Userdata、Thread、LuaState roots 与 Runtime roots；门拒绝 raw managed-edge
   setter、旧 `gc_step_remaining/gc_stopped` 和缺失 phase/root anchors。
2. `GarbageCollector::with_mut` 成为统一 `MutationContext`：先验证
   address/ObjectId/type，再执行写入并发布 post-write barrier；cross-heap/stale
   继续 fail-closed，black→white、weak-value 与 Thread→StateHandle 路径有定向回归。
3. collector 实现 Pause→Propagate→Atomic→Sweep→Finalize、debt/threshold、
   pause/stepmul、stop/restart、active-allocation initial graph 与有界 intrusive
   sweep cursor；Runtime 持有跨调用 StateHandle/object 双队列和 Atomic wide
   re-scan。`collectgarbage("step")` 不再使用倒计时，大步一次完成，小步按真实
   state/object/sweep unit 推进。
4. explicit incremental cycle 与 full collection 共用 weak-mode reconcile、
   finalizer resurrection、unreachable-state prepass 和 protected callback
   delivery；weak 清理、finalizer exactly-once、五阶段顺序、控制参数旧值和
   barrier-published coroutine state 均有回归。
5. allocation-triggered collection 按计划继续禁用；当前 debt/threshold 只为
   explicit step 与诊断服务，不夸大为 allocator live/peak 合同。

#### 16.3.8 已完成主任务：allocator accounting 与 automatic GC gate

1. allocator live/peak/total 已覆盖对象动态容器、StringPool key、StateArena
   与 shutdown 归零；
2. collector estimated bytes 与 managed allocator bytes 已分栏，automatic
   collection 只在 Runtime instruction boundary 推进；
3. stop/restart、weak/finalizer、explicit/automatic cycle ownership 与 10/10
   allocator contract 已通过。

#### 16.3.9 已完成本地切片：allocator failure 与 sanitizer durability

1. GC object、StringPool key、publication root、StateArena slot 四点 one-shot
   failure injection 已在 owner graph mutation 前接线，失败自动解除以允许
   rollback/shutdown/retry；
2. 新增 3 个 core fault tests、2 个 Runtime durability tests 和 1000-cycle
   pending-state soak，关闭后 object/root/string/state/allocator/queue 均为零；
3. 本地 `nightly-2026-07-29` Miri 已全部通过；首次运行曾发现 GC pointer
   retag 与 StateArena backpointer alias 两处 UB，现分别通过 raw ownership
   transfer 后派生指针及 pinned `UnsafeCell<StateArena>` 修复；
4. 本机 Visual Studio 已安装 `clang_rt.asan_dynamic-x86_64.dll`，将其
   `Hostx64/x64` 目录临时加入 `PATH` 后，Windows ASan 已通过 3 个 core、
   2 个 Runtime cases 与 1000 轮组合生命周期矩阵；未安装额外组件；
5. Linux Miri/ASan workflow 保留但按当前 Windows 环境策略延期，不将其
   伪造为本地证据；
6. 新增 1000 轮 production coroutine create/resume/drop、closure/upvalue、
   weak value 与 Lua `__gc` 组合矩阵，1000 个 finalizer 恰好执行，close 后
   state/object/root/string/allocator/queue 均归零；
7. Debug/Release 各 827 个 workspace tests、fmt、all-targets check/Clippy、
   warning-free rustdoc、10/10 allocator、24/24 root、8/8 mutation、
   17-path string 与 54-path heap contract 均通过；
8. binary dump 生命周期等待 M3 serializer，不在本切片伪造证据。

#### 16.3.10 已完成主任务：debug/protected-helper 跨 state open-Upvalue

1. `debug.getupvalue/setupvalue` 已改为 sealed `RuntimeNativeFunction`；关闭或
   requester-local 上值仍在当前 turn 完成，远端 open-Upvalue 通过 boxed
   `DebugUpvalueRequest`、deferred native frame 与 Runtime owner turn 读写；
2. `pcall(debug.getupvalue/setupvalue, ...)` 会 retarget 为 protected 回包；
   owner generation/Runtime 校验失败先转为 owned error response，再由 requester
   state 构造 Lua error value，不让 stale/foreign handle 逃出调度器；
3. debug request 的 Upvalue、name、write value、continuation snapshot 与 response
   已进入 `COROUTINE_ACTIVATION_BUFFER` canonical root seed，driver delivery 后
   exact pop，session unwind/close 清空新 transfer buffer；
4. boxed request 避免扩大所有 `LuaState` 的 dormant mailbox 尺寸；首次未盒装
   实现改变 allocator/automatic-GC cadence 并使 1000 轮 finalizer 矩阵暴露
   1001 次回调，盒装后定向与完整 Debug/Release 均恢复 exactly 1000；
5. direct/protected cross-state read/write、普通 closure 观察写值、owner 完成后
   closed access、foreign/stale response、close-before-invalidate 与 shutdown
   归零已有回归；Windows ASan 通过两条新增路径；
6. 当前 Windows 门禁：Debug/Release 各 829 项，fmt、all-targets check/Clippy、
   warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、17-path
   string、54-path heap 与 `git diff --check` 全部通过。Linux 验证继续按当前
   环境策略延期。

#### 16.3.11 已完成主任务：M1.5 深链与 broader coroutine fault 矩阵

1. 新增同一 open-Upvalue owner 下的 1000 层 `resume` 与 1000 层 `wrap`
   组合回归；普通 closure GET/SET 与 `debug.getupvalue/setupvalue` owner turn
   均在最深层发生，恢复顺序与固定 C++ oracle 的 `1999/2019/1020` 结果一致；
2. 新增 `RuntimeActivationStats`，实测峰值 resume frame=1000、普通/debug
   transfer 各=1、总 rooted activation=1001、同时 borrowed StateArena slot=1；
   Lua 主动 full collection 后 coroutine state=0，成功返回后 activation
   buffer=0；
3. 新增三层 `A→B→C→A` `Normal` 祖先与跨 state open-Upvalue owner-turn
   characterization，固定 C++ 的 replay/error、side-effect 与恢复顺序；
4. mailbox publication 与 deferred seal 后的 panic 注入均由 scope guard 清空
   owned request/snapshot，且同一 `LuaState` 可立即重新 publish/seal/take；
5. Runtime 新增 one-shot `SealedMailboxTake`、`UpvalueOwnerResolve`、
   `UpvalueResponseDelivery`、`ActivationUnwind` 与 `ShutdownPreflight` 耐久
   注入点；三层 unwind 后所有 activation/transfer buffer 归零，close 时
   Thread/state/open-Upvalue 链无 owner、stack 或 stale-edge mismatch；
6. shutdown preflight 注入发生在任何 finalizer/owner mutation 前，首次 close
   保持 Running、state count 与 allocator live 不变，one-shot 清除后重试
   close 全部归零。
7. 当前 Windows 门禁：Debug/Release 各 838 项 workspace tests、4 项
   doc-tests、fmt、all-targets Clippy、warning-free rustdoc、24/24 root、
   8/8 mutation、10/10 allocator、17-path string、54-path heap 与
   `git diff --check` 全部通过；`cargo-audit` 未安装而显式跳过，Linux
   Miri/ASan 继续按环境策略延期。
8. PowerShell 7 下显式指向固定 `lua_cpp@87c15e6` checkout 的 M1 smoke 为
   `checksPassed=true`、`hardFailures=[]`，raw-byte differential 与 artifact
   校验通过；因显式跳过已单独完成的 quality gate/audit 且 M1 尚未完成，
   `foundationPassed=false`、`m1Complete=false`。

#### 16.3.12 已完成切片：M1.5 arbitrary-Lua `pcall/xpcall` suspension

1. `pcall/xpcall` 已改为 sealed `RuntimeNativeFunction`，通过
   `ProtectedCallRequest` 携带 function、arguments、可选 error handler 与
   deferred caller snapshot，不再在 C helper 内同步递归执行 Lua；
2. Runtime activation 显式记录 target/handler phase、protected call boundary、
   parent driver role 与 pending response；成功、Lua error、handler error 和
   `error in error handling` 均在交付 outer deferred frame 时生成 Lua 5.1
   结果 envelope；
3. coroutine resume/wrap、ordinary/debug open-Upvalue、explicit/automatic GC
   与 nested protected call 的 delivery 都保留当前 protected role，释放当前
   state turn 后再恢复；128 层 nested `pcall` 的峰值 protected activation
   为 129，同时借用 state slot 始终为 1，结束后所有 buffer 归零；
4. 新增 Runtime-native target+handler、ordinary cross-state Upvalue
   target+handler 与 nested protected activation 三组回归；既有 direct
   protected resume/debug、错误身份和 `xpcall(error, error)` 语义保持通过；
5. 当前 Windows 门禁：Debug/Release 各 838 项 workspace tests、4 项
   doc-tests、fmt、all-targets Clippy、warning-free rustdoc、24/24 root、
   8/8 mutation、10/10 allocator、17-path string、54-path heap、固定
   C++ Normal characterization、raw-byte comparator 与 M1 smoke 全部通过。

#### 16.3.13 已完成主任务：其余同步 callback helper suspension

1. `tostring/__tostring`、多轮 `load` reader、`table.sort` comparator、debug hook、
   VM opcode metamethod、`dofile`、`table.foreach[i]`、package loader/module option 与
   `string.gsub` function/table replacement 均已迁移到 typed Runtime callback-helper activation；
2. 每类 helper 均显式保存结果发布、循环游标与错误策略；跨 turn 不保存 Rust `FnOnce`
   callback 或活跃 `&mut LuaState` 借用。生产 `call_value_with_results` 仅剩 Runtime close-time
   finalizer drain，继续作为 M1.12 已登记的 Runtime-native/cross-state fail-closed 边界；
3. 已完成的 `pcall/xpcall` 与 sealed debug get/setup continuation 保持专用
   typed envelope，不以通用化为由退回递归 state borrow。

#### 16.3.14 已完成切片：`tostring/__tostring` callback suspension

1. `tostring` 已由普通 C helper 改为 sealed `RuntimeNativeFunction::ToString`；无
   metamethod 路径仍在当前 turn 返回，`__tostring` 路径发布独立
   `CallbackHelperRequest`，不复用 `ProtectedCallRequest` 的 Lua-visible envelope；
2. Runtime 新增 callback-helper activation、root seed、峰值/当前深度统计和跨
   protected/helper 类型的单调 sequence 排序；任意 `pcall→tostring`、
   `tostring→pcall` 与嵌套 `tostring` 都恢复到最近的真实 parent driver role；
3. 通用 Runtime callback target 接受显式 wanted-results，`tostring` 固定收取首个
   返回值；Lua error 与禁止跨 C 边界的真实 yield 通过 typed response 回到原 native
   frame，成功/失败 delivery 后 exact pop；
4. 新增嵌套 `pcall→tostring→pcall→tostring` 回归，在最深 `__tostring` 内完成
   Lua-visible full collection 和 suspended coroutine 的 ordinary open-Upvalue 访问；
   结果为 `outer:inner:42`，protected/helper 峰值各 2、Upvalue transfer 峰值 1、
   同时借用 StateArena slot=1，执行后 activation buffer=0；
5. 新增 metamethod error→outer `pcall` 回归并验证错误可见、helper/protected buffer
   清空及 Runtime close 后 object/state/allocator live 全为零；
6. 当前 Windows 门禁：Debug/Release 各 840 项 workspace tests、4 项 doc-tests、
   fmt、all-targets check/Clippy、warning-free rustdoc、24/24 root、8/8 mutation、
   10/10 allocator、17-path string、54-path heap 与 `git diff --check` 全部通过；
   显式指向固定 `lua_cpp@87c15e6` checkout 的 M1 smoke 为
   `checksPassed=true`、`hardFailures=[]`，raw-byte differential 与 artifact 校验通过；
7. 本节记录的下一任务（`load` reader 多轮 typed continuation）现已由 16.3.15
   完成，table comparator 也已由 16.3.16 完成；debug hook 继续沿用同一
   scheduler substrate，但保留专用事件、结果与错误策略。

#### 16.3.15 已完成切片：`load` reader 多轮 callback suspension

1. `load` 已改为 sealed `RuntimeNativeFunction::Load`；string source 仍在当前 turn
   同步编译，function reader 则发布 `CallbackHelperKind::LoadReader` 并逐轮调用；
2. typed activation 自有 `chunk_name`、累计 `source: Vec<u8>` 与可选 terminal error；
   非空字符串片段继续复用同一 helper activation，`nil` 或空串终止，不跨 turn
   保存 Rust `FnOnce` callback 或活跃 `&mut LuaState` 借用；
3. byte-oriented chunk compiler 已从 stdlib 下沉为 VM 共享入口；终止后在原 deferred
   native frame 中直接 publication/compile。reader Lua error 返回 `nil,error`，非字符串
   返回 `nil,"reader function must return a string"`，确定性早期语法错误停止继续读；
4. 新增 `pcall→load→reader→pcall→tostring→collectgarbage("collect")` 回归；reader
   closure 访问 suspended coroutine 的 open-Upvalue，最终编译结果为 42，helper/protected
   峰值各 2、ordinary Upvalue transfer 峰值 1、同时借用 StateArena slot=1，完成后
   activation buffer=0；
5. 新增 reader Lua error、非字符串返回与跨多片 source bytes `[00,80,ff]` 回归；错误
   均按 `load` 结果合同交付，任意字节不经 UTF-8 重编码，Runtime close 后
   object/state/allocator live 全为零；既有 chunk-name arbitrary-byte 测试改由正式
   `Runtime::execute_proto` dispatcher 驱动并保持通过；
6. 当前 Windows 门禁：Debug/Release 各 843 项 workspace tests、4 项 doc-tests、
   fmt、all-targets check/Clippy、warning-free rustdoc、24/24 root、8/8 mutation、
   10/10 allocator、17-path string、54-path heap 与 `git diff --check` 全部通过；
   显式指向固定 `lua_cpp@87c15e6` checkout 的 M1 smoke 为
   `checksPassed=true`、`hardFailures=[]`，raw-byte differential 与 artifact 校验通过；
7. 本节记录的下一任务（`table.sort` comparator typed continuation）现已由
   16.3.16 完成；debug hook 继续沿用同一 scheduler substrate，但保留其事件、
   错误与目标寄存器策略。

#### 16.3.16 已完成切片：`table.sort` comparator callback suspension

1. `table.sort` 已由普通 C helper 改为 sealed `RuntimeNativeFunction::TableSort`；纯
   number/string 默认比较仍可在当前 turn 完成，Lua comparator 与默认 `__lt` 则逐次
   发布 `CallbackHelperKind::TableSort`；
2. boxed `TableSortState` 自有目标 table、comparator/type-metatable roots、values working
   copy、scratch、bottom-up merge 游标与 pending comparator phase；canonical root seed
   覆盖两份值数组和所有函数/表/string edge，不跨 turn 保存 Rust callback 或 state borrow；
3. 排序阶段对齐固定 C++ oracle：custom comparator 先逐元素 self-check，每次 truthy primary
   comparison 再做 reverse-check，并以 `invalid order function for sorting` 拒绝 hostile
   comparator；`{3,1,2}` 的 9 次 comparator 参数 trace 与固定 oracle 逐项一致；
4. comparator Lua error 保留原 error-value identity；排序只在全部 comparison 成功后一次性
   写回 working copy，失败/invalid-order 不发布部分结果，成功写回后对所有 collectable
   value 执行 post-write barrier；默认 `__lt` 使用 VM 相同的 common-metamethod 规则；
5. 新增跨 state open-Upvalue comparator 回归，覆盖
   `pcall→table.sort→comparator→pcall→tostring→collectgarbage("collect")`，并新增默认
   `__lt` 同构调度与失败清理回归；helper/protected 峰值各 2、ordinary Upvalue transfer
   峰值 1、同时借用 StateArena slot=1，成功/失败后 activation buffer=0，Runtime close
   后 object/state/allocator live 全为零；
6. 当前 Windows 门禁：Debug/Release 各 847 项 workspace tests、4 项 doc-tests、
   fmt、all-targets check/Clippy、warning-free rustdoc、24/24 root、8/8 mutation、
   10/10 allocator、17-path string、54-path heap 与 `git diff --check` 全部通过；
   显式指向固定 `lua_cpp@87c15e6` checkout 的 M1 smoke 为
   `checksPassed=true`、`hardFailures=[]`，raw-byte differential 与 artifact 校验通过；
7. 本节记录的下一任务（debug hook typed Runtime continuation）现已由 16.3.17 完成；
   其余 `call_value_with_results` 生产调用点也已重新审计并记录在该节。

#### 16.3.17 已完成切片：debug hook callback suspension

1. debug hook 不再同步递归调用 Lua；`CallbackHelperKind::DebugHook` 自有 event 参数、
   caller boundary 与 boxed `DebugHookContinuation`，覆盖 Lua call entry、精确 instruction
   PC、count 后 line 判定、Lua return/tail-return、native call 与 native return；
2. instruction pre-hook 以 exact-PC resume 避免重复计数或重复 line，Lua call hook 使用独立
   entry continuation，因而不会跳过 callee 首条指令；count hook 改为按每条解释器指令计数，
   同一 PC 同时触发时严格先 count 再 line；
3. Lua RETURN 的 `CallInfo`、collectable results 与 tail-return 余量，以及 native frame 的
   caller Proto/PC、wanted results 与 `DeferredVmContinuation` 均跨 turn owned/rooted；
   Runtime-native deferred completion 可先执行 return hook，再回到 CALL/TFORLOOP 原 continuation；
4. hook active 标志跨 Lua/native callback、嵌套 `pcall→tostring→collectgarbage("collect")` 与
   hook replacement 保持递归抑制；Lua error 保留原 error-value identity，并由真实 parent
   protected/program role 分类，成功、失败与 Runtime close 后 activation/object/state/allocator
   全部归零；
5. 新增 call/return/tail-return/line/count、同 PC count→line、collectable Lua/native return
   result 在 hook 内 full GC、coroutine 当前 state 安装 hook 后访问 main-state open-Upvalue，
   以及 self-clear/replacement/error identity 四组回归；callback-helper 峰值至少 2、protected
   峰值至少 1、ordinary Upvalue transfer 至少 1，同时借用 StateArena slot 始终为 1；
6. 当前 Windows 门禁：Debug/Release 各 851 项 workspace tests、4 项 doc-tests、fmt、
   all-targets check/Clippy、warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、
   17-path string、54-path heap 与 `git diff --check` 全部通过；固定
   `lua_cpp@87c15e6` Normal characterization、raw-byte comparator 与 M1 smoke 通过，smoke
   为 `checksPassed=true`、`hardFailures=[]`，M1 未完成故
   `foundationPassed=false`、`m1Complete=false`；
7. 生产 `call_value_with_results` 审计剩余 12 个调用点：1 个 close-time finalizer 维持已记录
   fail-closed 边界，4 个 VM opcode-local metamethod helper（`__index/__newindex`、算术/concat/
   len、比较）是下一主任务，另 7 个位于 `dofile`、`table.foreach[i]`、`string.gsub` 与
   package loader/module option；M1.5 callback-helper 主任务因此仍为 active。

#### 16.3.18 已完成切片：VM opcode metamethod callback suspension

1. `GETGLOBAL/GETTABLE/SELF`、`SETGLOBAL/SETTABLE`、算术/一元负号、`LEN`、`CONCAT`
   与 `EQ/LT/LE` 的 Lua metamethod 不再通过同步 `call_value_with_results` 递归执行；
   `CallbackHelperKind::VmMetamethod` 自有 caller boundary、callback scratch 位置和 typed
   `VmMetamethodContinuation`，Runtime 回包后恢复真实 parent program/protected/helper role；
2. continuation 显式覆盖五类 opcode 合同：普通首结果写回、`SELF` 方法结果加 receiver
   寄存器恢复、无结果 SET 完成、带 expected/invert 的比较跳转，以及携带剩余绝对 slot
   游标的多轮 `CONCAT` fold；Proto、SELF receiver、function/arguments 与 callback response
   均进入 canonical activation root，不跨 turn 保存 Rust 调用栈或活跃 state borrow；
3. `LEN` 对 string/table 保持 Lua 5.1 原始长度，对其他值新增 `__len` callback 回退并在缺失
   metamethod 时返回正式类型错误；`LE` 缺失 `__le` 时通过 swapped `__lt` 加 truthiness
   反转恢复，比较回包只按 opcode A 位决定是否跳过下一条指令；
4. 新增跨 state open-Upvalue 压力回归，一次覆盖 `__index/__newindex`、全局环境读写、SELF、
   `__add/__unm/__concat/__len/__eq/__lt/__le` 与 `__lt` fallback；每次 metamethod 内执行
   `pcall→tostring→collectgarbage("collect")`，callback-helper 峰值至少 2、protected 峰值
   至少 1、ordinary Upvalue transfer 至少 1、同时借用 StateArena slot=1，结束后所有
   activation 清零且 Runtime close 后 object/state/allocator live 全为零；另有错误对象
   identity→outer `pcall` 与失败清理回归；
5. 当前 Windows 门禁：Debug/Release 各 853 项 workspace tests、4 项 doc-tests、fmt、
   all-targets Clippy、warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、
   17-path string、54-path heap 与 `git diff --check` 全部通过；固定
   `lua_cpp@87c15e6` Normal characterization、bytecode/trace runner self-test、raw-byte
   comparator 与 M1 smoke 通过，smoke 为 `checksPassed=true`、`hardFailures=[]`，M1 未完成
   故 `foundationPassed=false`、`m1Complete=false`；
6. 生产 `call_value_with_results` 审计现剩 8 个调用点：1 个 close-time finalizer 继续维持
   已记录 fail-closed 边界；另 7 个位于 `dofile`、`table.foreach[i]`、`string.gsub` 的
   function/table replacement，以及 package loader/module option，成为下一主任务。

#### 16.3.19 已完成切片：`dofile` 与 `table.foreach[i]` callback suspension

1. `dofile` 已改为 sealed `RuntimeNativeFunction::DoFile`：文件读取与 byte-oriented 编译仍在
   当前 state turn 完成，编译所得 closure 通过 `CallbackHelperKind::DoFile` 交给 Runtime；
   callback 成功时转发全部返回值，失败时保留原始 Lua 错误对象 identity；
2. `table.foreach/foreachi` 已改为 sealed Runtime-native helper；typed `TableForeachState`
   分别保存 `next` key 游标或 numeric index/limit 游标，同时将 table、callback 与 pair key
   纳入 activation root。每轮 callback 的首个非 nil 结果立即结束，nil 则由同一 activation
   推进下一游标，耗尽后返回 nil；跨 turn 不保存 Rust closure 或活跃 `&mut LuaState`；
3. callback-helper 启动合同新增 `wanted_results`：`dofile` 请求全部结果，其余单结果 helper
   显式请求一个结果。VM 的 `TAILCALL` 新增 typed `DeferredVmContinuation::TailCall`，Runtime-native
   request 完成后关闭 caller upvalue、按 caller 结果合同搬运结果并弹出 caller frame，避免
   `return dofile(...)` 重跑原 Proto 或退回同步调用；
4. 新增 `dofile` 多返回与 tail-return、`table.foreach[i]` 游标/首个非 nil 语义回归；两组均在
   callback 内覆盖 `pcall→tostring→collectgarbage("collect")`、跨 state open-Upvalue、错误
   identity 与成功/失败 cleanup，完成后 activation/buffer 清零且 Runtime close 后
   object/state/allocator live 全为零；
5. 当前 Windows 门禁：Debug/Release 各 855 项 workspace tests、4 项 doc-tests、fmt、
   all-targets Clippy、warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、
   17-path string、54-path heap 与 `git diff --check` 全部通过；固定
   `lua_cpp@87c15e6` Normal characterization、bytecode/trace runner self-test 与 M1 smoke
   通过，smoke 为 `checksPassed=true`、`hardFailures=[]`，M1 未完成故
   `foundationPassed=false`、`m1Complete=false`；
6. 生产 `call_value_with_results` 审计现剩 5 个调用点：1 个 close-time finalizer 继续维持
   已记录 fail-closed 边界；`string.gsub` function/table replacement 各 1 个，package
   require loader 与 module option 各 1 个，成为下一主任务。

#### 16.3.20 已完成切片：package loader/module option callback suspension

1. `require` loader 不再通过同步 `call_value_with_results` 执行；
   `CallbackHelperKind::PackageRequire` 保存 `package.loaded` Table 与任意 byte-string module
   key，loader 成功后按 Lua 5.1 合同发布首个非 nil 结果、尊重 loader 自行写入的缓存值，
   仍为 nil 时写入 `true`，loader 失败则保留原始错误对象 identity；
2. `module` option 改为 `PackageModuleOptionsState` 的 ordered continuation：module Table、完整
   option 列表和下一游标均进入 activation root，每轮请求 0 个结果；前一 option 成功后才验证
   下一 option，因而保留“前序副作用已经发生、后续非函数才报错”的现有时序，任一 callback
   失败均停止后续 option 并传播原始错误对象；
3. stdlib 通过窄化的 VM request bridge 发布 sealed package helper，Runtime 独占回包、缓存写入、
   write barrier 和下一 callback 调度；跨 turn 不保存 Rust closure、文件 loader 临时 stack 根或
   活跃 `&mut LuaState`，compiled loader 由 deferred snapshot/root 接管并在完成时统一收栈；
4. 新增组合回归覆盖嵌套 preload `require`、缓存命中、nil→true、两轮 module option、后续非法
   option 时序、require/module 错误 identity，以及 callback 内
   `pcall→tostring→collectgarbage("collect")` 和跨 state open-Upvalue；峰值 callback-helper≥3、
   protected≥1、ordinary Upvalue transfer≥1、同时借用 state=1，结束后 activation/buffer 清零，
   Runtime close 后 object/state/allocator live 全为零；
5. 当前 Windows 门禁：Debug/Release 各 856 项 workspace tests、4 项 doc-tests、fmt、
   all-targets Clippy、warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、
   17-path string、54-path heap 与 `git diff --check` 全部通过；固定
   `lua_cpp@87c15e6` Normal characterization、bytecode/trace runner self-test 与 M1 smoke
   通过，smoke 为 `checksPassed=true`、`hardFailures=[]`，M1 未完成故
   `foundationPassed=false`、`m1Complete=false`；
6. 生产 `call_value_with_results` 审计现剩 3 个调用点：1 个 close-time finalizer 继续维持
   已记录 fail-closed 边界；另 2 个是 `string.gsub` function replacement 与 table replacement
   `__index` callback，成为下一主任务。

#### 16.3.21 已完成切片：`string.gsub` replacement callback suspension

1. `string.gsub` 的 function replacement 与 replacement Table `__index` function 不再通过同步
   `call_value_with_results` 执行；stdlib pattern engine 先生成只依赖 source/pattern 的 immutable
   match steps，保存每段 prefix/matched bytes、capture arguments 或 table key，Runtime 负责逐轮
   replacement 回包、拼接 output、计数和 suffix 发布；
2. `GsubState` 将 replacement Function/Table、canonical `__index` key、全部 collectable capture/key、
   next cursor、部分 output 与 suffix 纳入 callback-helper activation root；function 每个 match 调用
   一次，Table 每轮动态读取 raw value 并沿最多 100 层 `__index` Table 链解析，仅在遇到 Function
   时挂起，因而前序 callback 对后续 replacement Table/metatable 的修改仍可见；
3. nil/false 回包保留原 match，string/number 按 byte/Lua number 格式拼接，其他类型转为明确
   `invalid replacement value`；零 replacement limit 与全 raw Table 路径可同步完成，首次 Lua
   callback 才发布 sealed request。callback 错误保留原对象 identity，chain/类型错误由 Runtime
   创建正式 Lua error value；
4. 新增 function 与 Table `__index` 多轮组合回归，每轮 callback 内执行
   `pcall→tostring→collectgarbage("collect")`，并通过 suspended coroutine closure 访问跨 state
   open-Upvalue；另覆盖 max=0、function/Table 错误 identity 与非法回包。峰值 callback-helper≥2、
   protected≥1、ordinary Upvalue transfer≥1、同时借用 state=1，结束后 activation/buffer 清零，
   Runtime close 后 object/state/allocator live 全为零；
5. 当前 Windows 门禁：Debug/Release 各 857 项 workspace tests、4 项 doc-tests、fmt、
   all-targets Clippy、warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、
   17-path string、54-path heap 与 `git diff --check` 全部通过；固定
   `lua_cpp@87c15e6` Normal characterization、bytecode/trace runner self-test 与 M1 smoke
   通过，smoke 为 `checksPassed=true`、`hardFailures=[]`，M1 未完成故
   `foundationPassed=false`、`m1Complete=false`；
6. 生产 `call_value_with_results` 现仅剩 1 个 Runtime close-time finalizer drain 调用；其
   Runtime-native/cross-state 重入已在 M1.12 明确登记为 fail-closed shutdown 边界，ordinary
   stdlib/VM callback suspension 主任务至此闭环。下一主任务转入 main-state StateArena ownership。

#### 16.3.22 已完成主任务：main-state StateArena ownership

1. `Runtime` 不再外置保存 `Option<Box<LuaState>>`；`StateArena` 现在独占 main 与 coroutine
   全部 State Box，Runtime 仅保留 distinguished main `StateHandle`。原 `attach_external`/
   `detach_external`、pointer-match 与 external/owned 双轨已删除；
2. `Runtime::main_state`、`parts_mut`、top-level args/results、执行 trampoline、canonical root
   trace、full/incremental collection 与 close finalizer 都解析同一 arena slot；direct/turn borrow
   继续 fail-closed 拒绝 active/foreign/stale/borrowed handle，并在 panic/guard Drop 后更新 allocator
   计账和释放 slot；
3. arena 以 `is_main` 区分生命周期角色但对所有 occupied slot 保持唯一 Box owner。公开 live/drained
   coroutine 计数不包含 main；不可达 state prepass 明确跳过 main，即使 root-tracer defect 也不会
   销毁主状态；shutdown preflight 验证 distinguished main 与所有借用/invariant，随后按 coroutine
   slot order→main→Thread→ordinary→fixed 关闭、推进/退休 generation 并释放；
4. 新增/扩展 main handle→稳定地址与 Runtime move identity、single occupied main slot、borrowed-main
   rejection、`ShutdownPreflight` fault 零变更/allocator 不变/同地址 retry、coroutine-before-main drain
   handle 序列及 stale-generation 回归；root inventory 的 `MAIN_STATE_ENTRY` 改为 `implemented`，RFC
   与 deviation log 已同步删除 external owner 债务；
5. 当前 Windows 门禁：Debug/Release 各 859 项 workspace tests、4 项 doc-tests、fmt、all-targets
   Clippy、warning-free rustdoc、24/24 root、8/8 mutation、10/10 allocator、17-path string、54-path
   heap、JSON parse 与 `git diff --check` 全部通过；固定 `lua_cpp@87c15e6` Normal characterization、
   bytecode/trace runner self-test 与 M1 smoke 通过；
6. M1 smoke 为 `checksPassed=true`、`hardFailures=[]`；显式跳过已独立完成的 quality/differential/audit
   且 M1 尚未完成，故 `foundationPassed=false`、`m1Complete=false`。M1.5 本地合同闭环，下一主任务
   转入 M1.8 显式 IO/module service drain。

#### 16.3.23 已完成主任务：explicit Runtime IO/resource drain

1. `StateArena` 新增 Runtime 级资源注册表；每项只保存不成 GC root 的 `ObjectId + static drain hook`。
   `LuaState` 在资源对象图发布前先执行 checked capacity reservation，发布后无分配地登记 exact identity；
   collector 仅通过 authoritative live table 按 identity/type 重新构造 scoped Userdata handle，stale/wrong-type
   不解引用；registry capacity 纳入共享 allocator live/peak/total 计账；
2. `Runtime::close` 顺序现为 shutdown preflight/fault checkpoint→持续 Lua `__gc` drain→按注册顺序显式
   resource drain→coroutine slot order→main state→Thread→ordinary→fixed。单个 resource hook 的
   `Failed` 或 panic 被隔离并计入 report；already-closed/stale 独立计数；恶意无限追加有 1,000,000 次上限，
   所有路径在 state/heap teardown 前释放 registry capacity；成功 close 仍幂等复用首轮 totals；
3. IO 的 stdin/stdout/stderr、`io.open/tmpfile/input/output` 与新建 `io.lines` file 都在 publication 前预留、
   publication 后登记；hook 复用 scoped file state/payload access，执行 flush、direct-handle take 与
   `__closed` 标记。`package.loadlib` 仍显式 unsupported，当前 package 没有 native module handle；未来
   module loader 必须复用本 registry 或新增独立 inventory provider；
4. `RuntimeCloseReport` 新增 attempted/drained/already-closed/stale/error 五项 resource 统计，旧
   `io_resource_drain_debt` 变为 false；`LIBRARY_LIVE_HANDLES` root inventory 改为 `implemented` 并明确
   object graph 才是真 owner、registry 不参与 tracing；allocator inventory 新增
   `RUNTIME_RESOURCE_REGISTRY`，由 10/10 扩为 11/11；M1 smoke 删除已关闭的 M1.8 open debt；
5. 新增 exact-id live/type/stale lookup、hook panic isolation/idempotence、resource-registry allocation
   failure 零变更重试、`ShutdownPreflight` fault 零 hook 调用且重试 exactly-once、Lua `__gc` 失败后真实
   文件内容仍由 explicit drain 刷盘，以及 1000 个取消 Lua finalizer 的 IO resources 全部显式关闭并
   allocator live=0 的回归；
6. 当前 Windows 门禁：Debug/Release 各 864 项 workspace tests、4 项 doc-tests、fmt、all-targets
   check/Clippy、warning-free rustdoc、24/24 root、8/8 mutation、11/11 allocator、17-path string、
   54-path heap、JSON parse 与 `git diff --check` 全部通过；固定 `lua_cpp@87c15e6` Normal
   characterization、bytecode/trace runner self-test 与 M1 smoke 通过；smoke 为
   `checksPassed=true`、`hardFailures=[]`，显式 skip quality/differential/audit 且 M1 未完成，故
   `foundationPassed=false`、`m1Complete=false`。M1.8 本地合同闭环，下一主任务转入 M1.7 通用
   non-string scoped access audit。

#### 16.3.24 已完成切片：通用 scoped read API + table 标准库迁移

1. 新增安全的 `LuaState::with_gc_ref<T, R>`：只从当前 state 的动态 VM context 取得 owner
   collector，再由 authoritative live table 校验 address、进程级 `ObjectId` 与 concrete tag；读取仅能发生在
   HRTB callback 内，返回类型不能携带对象借用。无 active context、foreign collector 与 sweep 后 stale handle
   都在对象内存被触碰前 fail-closed；
2. table 标准库的 `getn`、`maxn`、`concat` 共 3 处 `GcRef<Table>::as_ref` 已迁移到该入口。
   `concat` 在 scoped callback 内只复制 `Value` 快照，字符串读取、结果分配和 Lua stack publication 都在
   Table borrow 结束后执行，不与 collector object borrow 重叠；
3. 新增 owner/no-context/foreign/stale 四路径单元断言，以及 `table_scoped_access_tests.rs` 静态守卫；既有
   `table_insert_remove_and_getn_work`、`table_concat_and_maxn_work` 行为回归通过；
4. 审计先观察到全仓 202 个 `.as_ref()` 文本命中；排除 AST、`Option`、`Pin`、`NonNull`、arena、测试和
   collector 已验证内部指针后，当前 `lua_vm + lua_stdlib` 精确生产直接 `GcRef` 解引用基线为 60：
   Table 20、Function 17、Proto 16、Userdata 4、Thread 2、Upvalue 1。VM dispatch 风险最大，但下一批先迁移
   base/package/debug 的 Function/Table 读取，以复用并稳定 API 形状，再进入 VM dispatch；
5. 当前 Debug/Release 各 867 项 workspace tests、4 项 doc-tests、fmt、workspace all-targets check/Clippy、
   warning-free rustdoc、2 项新增 API 定向 Miri、24/24 root、8/8 mutation、11/11 allocator、17-path
   string 与 54-path heap contract 通过；M1 smoke 为 `checksPassed=true`、`hardFailures=[]`，显式 skip
   quality/differential/audit 且 M1 未完成，故 `foundationPassed=false`、`m1Complete=false`。M1.7 仍为
   `partial`，不得据此声称 M1 或完整 scoped-access 合同完成。

#### 16.3.25 已完成切片：base/package/debug Function/Table scoped read

1. 审计将本批固定为 16 处：base 的 6 个 Table + 1 个 Function、package 的 2 个 Table + 1 个
   Function、debug 的 2 个 Table + 4 个 Function；Proto/Userdata/Thread 不混入本批，保持独立验收边界；
2. base 的 `unpack` 在 scoped Table callback 内生成 `Value` 快照，回调结束后才扩展 Lua stack；
   `rawget/getmetatable/setmetatable/next/ipairs` 与 function environment 读取均走 `with_gc_ref`。
   package 在已有 collector 的路径使用 `GarbageCollector::with_ref`，当前 C Function environment 使用
   state-scoped API；`table_get` 不再接受未经验证的裸句柄；
3. debug 的 C/Lua Function 分类、upvalue 数量、environment 与全局函数名查找均迁移。函数名查找在
   Table scope 内只返回 canonical `GcString` 句柄，Table borrow 结束后再走 string-scoped bytes，避免嵌套
   对象借用或借用逃逸；
4. 新增 `stdlib_scoped_access_tests.rs`，对三文件 Function/Table 直接访问建立静态守卫；既有
   `unpack`、raw/metatable、numeric `setfenv`、package module/preload 与 global-function `debug.getinfo`
   定向行为回归通过，完整 stdlib all-targets Debug/Release 通过；
5. 精确生产基线由 60 降至 44：Table 10、Function 11、Proto 16、Userdata 4、Thread 2、Upvalue 1。
   当前 Debug/Release 各 868 项 workspace tests、4 项 doc-tests、fmt、workspace all-targets Clippy 与
   warning-free rustdoc 通过。M1.7 仍为 `partial`；下一批先清零剩余 stdlib，再进入 VM dispatch。

#### 16.3.26 已完成切片：stdlib managed-object direct access 清零

1. 迁移剩余 20 处：Proto 8、Table 5、Userdata 4、Thread 2、Function 1。base/debug 不再把已验证
   Proto 降级为 `NonNull<Proto>`；traceback/getinfo/getlocal/setlocal/call-site name 只在 scoped callback
   内读取，并复制 source handle、line、active-line 或 `LocVar` 元数据后再访问 stack/string；
2. coroutine/debug 的 Thread 读取先复制 `CoroutineStatus` 或 `StateHandle`，borrow 结束后再输出或解析
   state。string.gmatch、os.time、IO iterator 将 Table 字段复制为拥有所有权的 `Value`/scalar 快照，随后
   才修改 Lua stack 或 table；
3. IO 的 `with_file_state/with_file_data` 通过 `LuaState::with_gc_ref` 逐跳校验 Userdata→metatable 与 typed
   payload；`with_file_data_mut` 改为 `GarbageCollector::with_mut`，删除 `GcRef::as_ptr` 强转成
   `*mut Userdata` 的未验证可变解引用，并让直接文件 handle mutation 使用同一 collector scope；
4. `stdlib_scoped_access_tests.rs` 扩成枚举 `lua_stdlib/src` 全部 12 个 Rust 文件，禁止生产行重新出现
   `unsafe ... .as_ref()/.as_mut()`。精确生产基线从 44 降至 24，全部位于 `lua_vm/src/execute.rs`：
   Function 10、Proto 8、Table 5、Upvalue 1；
5. 当前 Debug/Release 各 869 项 workspace tests、4 项 doc-tests、fmt、workspace all-targets Clippy 与
   warning-free rustdoc 通过；2 项全源静态守卫在独立 Miri target 下通过。全量 stdlib lib Miri 在进入
   本批 IO/debug 路径前，被既有 `with_vm_context_parts` closure 的 StringPool `&mut` 与动态 context
   重构 `&mut` 叠加所拒绝；该结果不记作本批失败或全量 Miri 通过，而与 VM dispatch 一并作为下一批
   alias-boundary 任务。该任务现已由 16.3.27 完成。

#### 16.3.27 已完成本地切片：VM scoped access 清零与 VmContext alias 收敛

1. `lua_vm/src/execute.rs` 的 24 处直接 managed-object 访问已全部迁移：Function 10、Proto 8、
   Table 5、Upvalue 1。指令、常量、stack size、函数分类、vararg、upvalue、metatable 和诊断元数据
   均只在 `GarbageCollector::{with_ref,with_mut}` 的短 callback 内读取，跨 callback 只保留
   `Value`、`GcRef` 或 scalar 快照；新增全文件静态守卫，禁止 `.as_ref()`/`.as_mut()` 回流；
2. 删除会在 closure 全程持有 `&mut GarbageCollector + &mut StringPool` 的
   `with_vm_context_parts`。`with_vm_context_gc` 现在把显式 collector 交给 callback，同时 fail-closed
   隐藏同一 collector 的动态恢复，但保持相互独立的 StringPool 服务；普通 C ABI callback 在实际
   调用边界建立由当前短 reborrow 派生的嵌套 context，回调结束后恢复外层隐藏状态；
3. stdlib 注册、package、IO、debug/runtime-native 参数读取和 VM 内部 string/table 查找在已有显式
   collector 时统一走显式 scoped API。该边界加固曾由回归捕获 `load` chunk name、`dofile` 文件名与
   `collectgarbage` 字符串参数的动态 GC 依赖，均已改为显式 collector 读取并保留 arbitrary bytes；
4. 当前 Debug/Release 各 871 项 workspace tests、fmt、all-targets check/Clippy、warning-free rustdoc、
   24/24 root、8/8 mutation、11/11 allocator、17-path string、54-path heap 与 `git diff --check`
   全部通过；本切片首次 smoke 为 `checksPassed=true`、`hardFailures=[]`，完整 foundation gate
   后续结果见 16.3.28；
5. pinned `nightly-2026-07-29` 下，全量 `lua_stdlib --lib` Miri 23/23 通过；VM 的真实普通 C callback
   重入与 `with_vm_context_gc` 隐藏动态 collector/保留 StringPool 两项定向 Miri 通过。nightly 对
   `Atomic::fetch_update` 的重命名提示不影响 stable `-D warnings` 门禁；
6. M1.7 标记为 `completed-local`，不代表 M1 整体完成。下一主任务按 16 节第 6 项执行完整 M0/M1
   quality/differential/audit 策略、raw-byte 双 oracle、fresh 101-case non-official 与代表性 bytecode
   parity，并更新精确 artifact/SHA；该刷新现已由 16.3.28 完成。

#### 16.3.28 已完成本地切片：完整门禁与 fresh oracle/artifact 刷新

1. 完整 M1 foundation gate 未跳过 quality、differential 或 audit，结果为
   `checksPassed=true`、`foundationPassed=true`、`m1Complete=false`、0 hard failure、1 项
   M1.13 跨平台 sanitizer/public allocator 债务，用时 24,672 ms；quality 子门包含 fmt、
   all-targets Clippy、Debug/Release 各 871 项测试、warning-free rustdoc 与 `cargo-audit 0.22.2`；
2. 首次 full differential 正确拒绝相邻开发工作树 `lua_cpp@14f5250`，因为计划固定
   `87c15e6`。未改动或回退用户工作树；改为使用独立
   `target/oracles/lua_cpp-source@87c15e6` 快照，并让 M1 gate 在未显式指定时优先该快照、环境变量
   仍可覆盖。重跑后 M1 raw-byte official/C++ 两 lane 各 2/2，完整 foundation gate 全绿；
3. 完整 M0 compatibility gate 重建 Rust、固定 C++ 与 Lua 5.1.5 工具，结果
   `passed=true`、`fullGatePassed=true`、0 hard failure、3 项已登记债务，用时 29,909 ms；focused
   dual-oracle 为 4/4×2，fixture manifest 现为 132，parity runner fail-closed self-test 通过；
4. fresh `non-official-m1-current.json` 生成于 `2026-08-11T14:43:09.851Z`，runner 0.1.0；
   selected 101、executed 92、helper 9、raw match 4、difference 88、runner error/timeout 0。
   attribution-only 复算显示 88 个 difference 中 raw+EOL 等价 71、剩 17，声明 normalization+EOL
   等价 74、剩 14；加 4 个 raw match 后 compared-channel 为 78/92，因
   `integration.loadfile-dofile-workflow` 与 `stdlib.iolib-core` side effect 仍不相等，
   complete-channel 为 76/92；
5. fresh executable SHA-256：Rust `lua_app` 为
   `089657459e017e8df73ad7a040d9dbadb20fbc1e12f2faaa467391e1415c88cb`，Rust
   `lua_bytecode` 为 `fb71e2eededd7975576078cd288aa09483235ce39181ba3cca219ffdac137c8a`，
   C++ `lua_app` 为 `bde546c0fac16cadf9dd3e6b161bf1ed84ba210ea3b2be1320da34cd34d63695`，
   C++ `lua_bytecode` 为 `ebf61574ff6c06d2539e5e05fb0297d189278ef86cbac1cd04684441d20e044b`，
   official Lua 5.1.5 为 `f3e44374b32b48af424ce46d7437b324ef0aca15e338466a1683f69da14366d7`；
6. representative bytecode 仍为 0/2，但 0 infrastructure failure；两例均只剩
   `evidence.localNames` 缺失与对应 `$.protos[0].localNames` null 两项，opcode、constant 和其余
   metadata 一致。下一主任务进入 M2 P0-B：先建立固定 C++ local-name 受审计证据，再从
   `test_closure_pipeline.lua` nested Proto 的首个真实分歧推进，不能把 500 条截断列表当成
   500 个独立根因。

#### 16.3.29 已完成本地切片：C++ local-name 证据与首个 nested Proto 分歧

1. 固定 `lua_cpp@87c15e6` 的 `Proto` 已确认保存完整 `LocVar` 名称、start/end PC 与 register，
   但 pristine `lua_bytecode full` 不输出 locals。新增 checkout 外的只读
   `tools/cpp_bytecode_evidence`，链接由固定 checkout 安装的 `LuaCpp::Lua`，仅使用
   `Proto` const accessor 递归输出 path、arbitrary-byte local name envelope 与 LocVar 元数据；
   不修改固定 checkout 或原始 `lua_app`/`lua_bytecode`；
2. `build_cpp_oracle.ps1` 现在同时构建 sidecar，并在 `cpp-oracle-build.json` 记录 executable、
   CMake 与 source SHA-256。fresh sidecar SHA 为
   `4bbd5ddf318d30022d93d8c1b1e6acc1192f892baf9188a79dc37cc503c604fe`；原 C++
   `lua_app`/`lua_bytecode` SHA 仍分别为
   `bde546c0fac16cadf9dd3e6b161bf1ed84ba210ea3b2be1320da34cd34d63695` 与
   `ebf61574ff6c06d2539e5e05fb0297d189278ef86cbac1cd04684441d20e044b`；
3. `compare_bytecode.ps1` 对 C++ lane 单独运行并保存 evidence 进程，校验 schema、固定 commit、
   byte envelope、LocVar 元数据和 exact Proto path coverage 后才补入 `localNames`；缺工具、坏
   schema、进程失败或路径不全均为 infrastructure failure。runner self-test 也显式验证左右
   evidence process artifact 与 `localNames=true`；
4. fresh `target/compatibility/bytecode-representative/report.json` 为 2/2 passed、0 difference、
   0 infrastructure failure。完整 M0 gate 为 `passed=true`、`fullGatePassed=true`、0 hard failure、
   2 项债务、20,682 ms；原 `representative-bytecode-differences` 债务已移除，只剩 88 个
   non-official 差异和 real VM trace unsupported；
5. closure artifact 的首个 nested 非行号分歧定位为 `0.children[0]` PC 0：
   `{ n = select("#", ...), ... }` 的 `NEWTABLE B` 在 C++ 为 0、Rust 为 1。根因是 Rust 将尾部
   open vararg/call 错误计入固定 array capacity hint；两分支均在发出 open `SETLIST` 后扣除该
   非固定字段，并新增 vararg/call 两个最小编译器回归。修复后 `0.children[0]` 与
   `0.children[1]` 的 instruction words 全部一致；
6. 完整 M1 foundation gate 在临时加入仓内既有 `cargo-audit 0.22.2` 路径后通过：
   `checksPassed=true`、`foundationPassed=true`、`m1Complete=false`、0 hard failure、22,223 ms；
   fmt、all-targets Clippy、Debug/Release 各 872 项测试、warning-free rustdoc、audit、全部静态合同与
   raw-byte 双 oracle 均通过。下一主任务从 `0.children[1].children[0]` PC 1 的
   assignment/register scheduling 分歧继续最小化，当前 C++/Rust 指令数为 13/15。

#### 16.3.30 已完成本地切片：nested assignment 与 relocatable arithmetic result

1. 将 `remember` child 的首差异分别最小化为 field、index 与 combined assignment；修复前
   C++/Rust 指令数依次为 7/8、7/8、13/15。根因是 Rust 先求 RHS 再求 lvalue，而固定 C++
   先按源码顺序求值并冻结全部 lvalue，随后求 RHS，最后逆序写回；这同时是 Lua 赋值副作用
   顺序的语义差异；
2. assignment lowering 现复刻固定 C++ 的 stable receiver/key 判定、必要的 lvalue 临时冻结、
   local-to-local fast path、双 local swap 与 reverse store；新增 structured bytecode 回归和
   `indexed_assignment_evaluates_lvalue_before_rhs` VM 行为回归。三个最小 child 均已
   word-identical；
3. arithmetic lowering 进一步改为先固定左 operand、再求右 operand，并返回 relocatable
   instruction，由 consumer patch 结果寄存器，不再提前占用结果寄存器；新增 upvalue arithmetic
   assignment 回归。修复后 `remember` 为 13/13、完整 pipeline closure 为 34/34，
   `0.children[0]`、`0.children[1]` 与 `0.children[3]` 也保持逐字一致；
4. 当前首个更深 word difference 已移动到 `0.children[2]` 的 `fold`：C++/Rust 为
   19/18 条指令。首个 word 差异是 PC 6 的 `FORPREP` 跳距；C++ 在 `FORLOOP` 前额外发出
   `CLOSE A=7`，Rust 没有。下一主任务固定为 numeric-for body scope close 判定，不回退到
   已关闭的 assignment/register 根因；证据见
   `target/compatibility/bytecode-closure-after-relocatable-arithmetic/report.json`；
5. 完整门禁曾捕获 1000 层 resume/wrap 深链在脚本内 GC 后仍有 1 个 coroutine state。
   审计确认固定 C++ 顺序使最后一个 wrapper/closure 留在 active frame 的 dead temp register，
   Lua 5.1 风格 conservative active-frame max-stack tracing 会合法保留该单一 state；测试现锁定
   GC 时为 1，并继续要求 `Runtime::close` 后 object/state/allocator 全部归零，未放松 shutdown
   生命周期合同；
6. 最新完整 M0 为 `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、22,535 ms；
   完整 M1 为 `checksPassed=true`、`foundationPassed=true`、`m1Complete=false`、0 hard failure、
   24,687 ms。fmt、all-targets Clippy、Debug/Release 各 875 项测试、warning-free rustdoc、audit、
   raw-byte 双 oracle 与全部静态合同通过。总计划、M1 与 nested compiler parity 仍未完成。

#### 16.3.31 已完成本地切片：for-loop iteration-scope CLOSE parity

1. `fold` 的 19/18 差异已最小化为固定 C++ 的显式 loop contract，而非 captured-upvalue
   分析：numeric-for body 非空时无条件发 `CLOSE(base+3)`；body 为空时不发。generic-for
   只要存在可见循环变量，就连空 body 也在 `TFORLOOP` 前发 `CLOSE(base+3)`；
2. Rust numeric/generic for lowering 已同步该合同；`emit_close` 提升为 codegen crate 内可用，
   仍统一 flush pending jumps。新增单个结构化回归同时锁定 numeric empty/non-empty、generic
   empty/non-empty 四种 opcode sequence 和 `CLOSE A`；修复前红测准确缺少 non-empty numeric
   的 `CLOSE`；
3. `test_closure_pipeline.lua` 的 `fold` 从 C++/Rust 19/18 收敛为 19/19。fresh
   `target/compatibility/bytecode-closure-after-for-close/report.json` 显示 6 个 child Proto
   的 instruction words 全部一致：`pack` 10/10、`makePipeline` 14/14、`remember` 13/13、
   pipeline closure 34/34、`fold` 19/19、`forward` 5/5；完整脚本在 Rust 运行时通过；
4. 根 Proto 仍为 C++ 198 / Rust 224，当前第一个 word difference 前移到 PC 40：源码
   `firstBatch[1] == 3 and ...` 在 C++ 中直接串接 comparison jump list，Rust 则逐段
   materialize `LOADBOOL/MOVE/TEST`。下一主任务固定为 short-circuit boolean
   condition/value lowering；
5. 第一次无参数 M0 调用正确拒绝相邻开发工作树 `lua_cpp@14f5250`，没有改动该工作树；显式
   `-CppRoot target/oracles/lua_cpp-source` 后，固定 `87c15e6` 的完整 M0 恢复为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、19,412 ms。完整 M1 为
   `checksPassed=true`、`foundationPassed=true`、`m1Complete=false`、0 hard failure、
   73,821 ms；fmt、all-targets Clippy、Debug/Release 各 876 项测试、warning-free rustdoc、
   audit、raw-byte 双 oracle 与全部静态合同通过。

#### 16.3.32 已完成本地切片：closure pipeline 完整 structured parity

1. 根 Proto PC 40 的 198/224 差异最小化为 boolean-only `and/or` lowering：固定 C++ 的
   `producesBoolean` 识别 bool、`not`、comparison 与完全布尔的嵌套 `and/or`，直接组合
   true/false jump list，最后只物化一对 `LOADBOOL`；Rust 原实现把每个 comparison 先物化成
   register，再发 `MOVE/TEST`。Rust 已复刻专用通道，同时普通 `a and b` / `a or b` 仍走
   value-preserving 通道；三 comparison 最小 case 从 24 条收敛为固定 C++ 的 11 条；
2. 新增精确 raw instruction 回归和带调用计数的 VM 副作用回归，锁定 `and`/`or` 都跳过不应
   执行的右侧。generic-for 初始 jump 同步修正为跳过 per-iteration `CLOSE`、直接落到
   `TFORLOOP`；numeric/generic 显式 `CLOSE` 的 line 恢复到 loop statement line；
3. boolean 修复后根 Proto 只剩隐式收尾差异。top-level block 现先按 level 0 移除 locals，
   对 captured root locals 发 `CLOSE A=0`，再追加隐式 RETURN；child 编译保存/恢复 parent
   `current_line`，隐式 RETURN 使用 `lastlinedefined`，while back edge 使用固定 C++ 的
   synthetic line 0。新增 root CLOSE/parent-child line 与 while line 两项结构化回归；
4. fresh `target/compatibility/bytecode-closure-full-parity/report.json` 首次达到
   `status=passed`、1/1 passed、0 difference、0 infrastructure failure。根 Proto 为
   198/198，6 个 child 为 10/10、14/14、13/13、34/34、19/19、5/5；opcode/raw word、
   operand/RK、constant order/value、nested Proto、function、line、local 与 upvalue evidence
   全部一致。该单个复杂 case 通过不代表 M2.2/M2.3 或 38-opcode parity 完成；
5. 最新固定 `lua_cpp@87c15e6` 完整 M0 为 `passed=true`、`fullGatePassed=true`、0 hard
   failure、2 debts、21,257 ms；完整 M1 为 `checksPassed=true`、`foundationPassed=true`、
   `m1Complete=false`、0 hard failure、73,578 ms。fmt、all-targets Clippy、Debug/Release
   各 886 项测试、warning-free rustdoc、audit、raw-byte 双 oracle 与全部静态合同通过；
6. 此处记录的下一入口已由 16.3.33 与 16.3.34 执行：先扩展 bounded structured corpus，
   再闭合 frontend-produced opcode 覆盖；88 个 non-official process 差异和真实 VM trace
   unsupported 仍保持开放。

#### 16.3.33 已完成本地切片：12-case bounded structured bytecode 全绿

1. 扩展 corpus 先暴露 helper workflow 的 table array scheduling、concat flattening 与
   `emit_value` line restore 差异；三项均按固定 C++ emitter 顺序修复并由精确 bytecode
   regression 锁定；
2. `test_coroutine_scheduler.lua` 的首个差异继续收敛出 member callee 临时寄存器、显式
   call-base move、local call multi-return frame 与 nil/boolean immediate RK constant 四项合同。
   修复后该 case 根 Proto 334/334、全部 6 个 child Proto 逐 word 与完整 metadata 一致；
3. fresh `target/compatibility/bytecode-integration-bounded-12-after-coroutine/report.json`
   为 5/5 passed、0 difference、0 infrastructure failure，共 23 个 Proto、1206 条 instruction、
   290 个 constant，覆盖 29/38 opcode；
4. whole-tree auto selection 同时发现 invalid syntax fixture 会让 bytecode printer 无 stdout。
   comparator 现将空输出作为 infrastructure failure fail closed；runner self-test 新增该负例，
   要求非零退出且完整生成 `infrastructure-failed` 报告；
5. 剩余有效 case `alien_signals/constants.lua` 的 instruction word 已一致，只差 nested direct
   call 的 10 项 line metadata。`emit_call_expr` 现复刻固定 C++ call-level LineGuard；该 case
   根 Proto 122/122 与 19 个 child 全部一致；
6. 新增显式正向 manifest `tests/compatibility/bytecode-structured-bounded-12.json`，排除
   invalid syntax 但保留其负向 self-test。fresh
   `target/compatibility/bytecode-structured-bounded-12/report.json` 为 12/12 passed、0 difference、
   0 infrastructure failure，共 64 个 Proto、1756 条 instruction、417 个 constant，覆盖
   30/38 opcode；与 integration corpus 并集为 34/38，尚缺 `UNM/NOT/LEN/TESTSET`；
7. 固定 `lua_cpp@87c15e6` 完整 M0 为 `passed=true`、`fullGatePassed=true`、0 hard failure、
   2 debts、21,257 ms；完整 M1 重跑为 `checksPassed=true`、`foundationPassed=true`、
   `m1Complete=false`、0 hard failure、1 open debt、73,578 ms。Debug/Release 各 886 项测试、
   fmt、all-targets Clippy、warning-free rustdoc、audit、双 oracle 与全部静态合同通过；
8. 此处记录的剩余 opcode 入口已由 16.3.34 完成：新增有效 `UNM/LEN` case，并确认固定
   C++ 的 `NOT/TESTSET` intentionally-not-produced 合同。88 个 non-official process 差异与
   真实 VM trace unsupported 仍保持开放，M2.2/M2.3 未完成。

#### 16.3.34 已完成本地切片：frontend-produced opcode 36/36 覆盖

1. 固定 `lua_cpp@87c15e6` 的可执行 opcode coverage contract 明确：38 个 VM opcode 中有
   36 个由 frontend 生成；`NOT` 与 `TESTSET` 均为 `intentionally-not-produced`。`not`
   表达式走 `TEST/JMP/LOADBOOL` 条件管线；`TESTSET` 只允许出现在将 `NO_REG` 规范化为
   `TEST` 的无 production caller guard；
2. 新增 `tests/lua/differential/m2-unary-frontend-coverage.lua`，用动态 operand 强制生成
   `UNM` 与 `LEN`，并用 value-form `not` 锁定不生成 `NOT` 的 oracle 合同。单例 structured
   bytecode 为 1/1 passed、0 difference，根 Proto 41/41、child Proto 2/2；
3. 新增 `tests/compatibility/bytecode-structured-bounded-13.json`。fresh
   `target/compatibility/bytecode-structured-bounded-13/report.json` 为 13/13 passed、
   0 difference、0 infrastructure failure，共 66 个 Proto、1799 条 instruction、426 个
   constant；本批单独覆盖 32/38 opcode；
4. 与 5-case integration corpus 合并后覆盖 36/38 opcode，恰好等于固定 C++ 合同的
   36/36 frontend-produced opcode；仅缺的 `NOT/TESTSET` 都有 oracle 的可执行负向合同，
   不再把源码 compiler parity 的完成条件错误表述为必须强造 38/38；
5. 新增 case 已登记为第 7 个 differential fixture；manifest 现为 132 项且 complete
   classification 校验通过。Rust/C++ 进程行为 1/1 match、0 difference，官方 Lua 5.1.5
   也以退出码 0 通过；
6. 首次完整 M0 暴露 fixture-runner 两个单测仍硬编码 131/6。测试现从 manifest 的
   `current_total/added_differential` 推导增长型期望，同时继续硬锁 non-official 101-case
   基线；runner 定向测试 9/9 通过；
7. fresh `target/compatibility/m2-opcode-coverage-final-m0-rerun/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、21,257 ms。本批未出现新的
   structured 差异，因此无需实现 compiler 代码修复。下一主任务是扩大下一组有效 corpus，
   并从新的首个真实差异继续；88 个 non-official process 差异与真实 VM trace unsupported
   仍保持开放，M2.2/M2.3 未完成。

#### 16.3.35 已完成本地切片：24-case structured corpus 与控制流 parity

1. 新增 `tests/compatibility/bytecode-structured-bounded-24.json`，在原 13-case 基础上加入
   basic、for/if、value-types、multiret/vararg、closure integration、lvalue/multret/short-circuit
   regression 与 metamethod arithmetic。首次结果为 20/24：1 项 comparator infrastructure
   failure 和 3 项 semantic failure；
2. `control_flow/test_if_logic.lua` 将首个真实差异最小化为括号条件：固定 C++ 在
   `emitCondResult/emitCondResultTrue` 中递归解包 `ParenExpr`，Rust 原实现落入值物化通道。
   Rust 已同步 jump-list 管线，并新增逐 instruction/line/max-stack 的精确红绿回归；
3. `differential/value-types.lua` 的 infrastructure failure 来自 PowerShell 对单元素
   `line_info` 数组的标量展开。comparator 现用 `[object[]]@(...)` 保持数组形态；runner self-test
   新增 Rust recursive singleton Proto，用一个 child、一个 line-info 项锁定 fail-closed 解析；
4. 剩余 `test_short_circuit_materialization.lua` 暴露两个固定 C++ statement contract：
   `if/elseif/else` 离开分支的 JMP 使用整个 `if` 的 `endLine`；`repeat` 条件刻意走
   value → single-value → register → `TEST` → back-edge，而不是通用条件 jump-list。Rust 已逐项
   同步，两个最小测试在修复前分别稳定红于 lineInfo 和 raw instruction sequence；
5. `repeat ... until true` 的无条件退出，以及 `false/nil` 的 `LOADBOOL/TEST/back-edge` 特例另有
   精确回归，并锁定 repeat-body 未初始化 local 每轮显式 `LOADNIL`。原
   `tables/test_metamethod_arith.lua` 的唯一行号差异随 `if endLine` 根因一并关闭，不是第三个根因；
6. fresh `target/compatibility/bytecode-structured-bounded-24-after-repeat-value/report.json` 为
   24/24 passed、0 difference、0 infrastructure failure，共 107 个 Proto、3295 条 instruction、
   765 个 constant；本批单独覆盖 36/38 opcode，即固定 C++ 的 36/36 frontend-produced opcode；
7. pinned `target/compatibility/m2-structured-24-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、19,624 ms。fresh M1 重验用时
   67,927 ms；静态合同、fmt、workspace Clippy、Debug/Release、warning-free rustdoc 与 raw-byte
   differential 全绿，唯一 hard failure 是本机未安装 `cargo-audit`，因此未把该次结果误记为
   foundation pass。完整 quality run 覆盖 893 项测试；随后新增的 repeat literal 特例在
   Debug/Release 定向通过，当前 workspace 枚举 894 项；
8. 下一主任务继续扩大 24-case 之外的有效 structured corpus；88 个 non-official process 差异、
   真实 VM trace unsupported 与 M1 的跨平台 sanitizer/public allocator 债务仍保持开放，
   M2.2/M2.3 和总计划均未完成。

#### 16.3.36 已完成本地切片：36-case structured corpus 与作用域/vararg parity

1. 新增 `tests/compatibility/bytecode-structured-bounded-36.json`，在 24-case 基础上加入
   alien-signals engine/graph/primitives/scheduler、nested break/if、functions、env loader、
   call/upvalue/value regressions 与 coroutine closures。首次结果为 31/36、5 项 semantic
   failure、0 infrastructure failure；
2. `alien_signals/engine.lua` 暴露 RETURN 临时寄存器基址泄漏：固定 C++ 在 return-list
   前把 free register 重置到 active-local 边界。Rust 已同步并用条件临时值后的双分支返回
   锁定精确 9 条 child 指令；
3. `alien_signals/graph.lua` 暴露局部 nil 初始化活性合同。Rust 现完整复刻函数入口寄存器
   默认 nil、后续不读取局部省略 `LOADNIL`、后续读取保留 line 0 `LOADNIL`、全 nil initializer
   合并，以及 repeat 每轮强制重置；入口/死局部/活局部与 repeat 均有定向回归；
4. `functions/test_functions.lua` 的唯一 metadata 差异来自 legacy vararg：未消费 `...` 时
   固定 C++ 同时设置 `HASARG | ISVARARG | NEEDSARG`，Rust 原来漏掉 `HASARG`。标志与隐式
   `arg` slot 现已一致；
5. `test_call_pipeline.lua` 的唯一 line-info 差异来自 `do` scope 自动 `CLOSE`：固定 C++ 使用
   block opening line，Rust 原来沿用末条 body line。`do` 退出行号已精确锁定；
6. `test_upvalue_close.lua` 将 468 个级联差异归并为 while/break 的一个根因：正常回边必须
   先关闭本轮捕获局部，`break` 还必须在跳出前单独 `CLOSE` 并跳过回边。Rust 现对两个出口
   分别发射与固定 C++ 相同的 `CLOSE`，原始回归单例已 1/1 passed；
7. fresh `target/compatibility/bytecode-structured-bounded-36-final/report.json` 为 36/36 passed、
   0 difference、0 infrastructure failure，共 235 个 Proto、7673 条 instruction、1746 个
   constant，覆盖 36/38 opcode，即固定 C++ 的 36/36 frontend-produced opcode；当前 workspace
   枚举 899 项测试；
8. pinned `target/compatibility/m2-structured-36-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、20,303 ms。fresh M1 重验
   `target/compatibility/m2-structured-36-final-m1-pinned-rerun/gate.json` 的静态合同、fmt、
   workspace Clippy、Debug/Release、warning-free rustdoc 与 raw-byte differential 全绿，唯一
   hard failure 仍是本机缺少 `cargo-audit`，用时 70,867 ms；
9. 下一主任务继续扩大 36-case 之外的有效 structured corpus；88 个 non-official process
   差异、真实 VM trace unsupported 与 M1 的跨平台 sanitizer/public allocator 债务仍开放，
   M2.2/M2.3 和总计划均未完成。

#### 16.3.37 已完成本地切片：48-case structured corpus 与 byte-safe/词法作用域 parity

1. 新增 `tests/compatibility/bytecode-structured-bounded-48.json`，在 36-case 基础上加入
   alien-signals example/init/tracer、break/while、vararg table、官方 calls/closure/constructs/
   locals、lvalue pipeline 与 coroutine generators。首次结果为 44/48、4 项 semantic
   failure、0 infrastructure failure；
2. `official/calls.lua` 暴露 pristine C++ full-text printer 对 embedded NUL string constant
   的有损输出。checkout 外 evidence sidecar 已升级到 schema 2，按 byte envelope 输出 string、
   按 IEEE-754 bits 输出 number，并与 full-text Proto exact-path/constant-count 合并；runner 自检
   新增 `"\0A"` 常量，任何 evidence 缺失、类型或长度不一致均 fail closed；
3. `official/locals.lua` 暴露 nonlocal function statement 的寄存器调度：固定 C++ 总是先在
   temporary register 创建 closure，再 `MOVE` 到 local。Rust 已同步，并以精确 4 条 root
   instruction 回归锁定；该修复也关闭了 `official/closure.lua` 中缺失的两个 `MOVE`；
4. `official/closure.lua` 继续暴露两类作用域差异：`break` 已自行关闭捕获局部，因此必须跳到
   loop final `CLOSE` 之后；`if` 自动 `CLOSE` 使用 statement line。两个目标/行号合同均已用
   精确 instruction 回归锁定；
5. 编译 block 现统一在词法边界恢复 statement line、移除本层 local 并按需发射 `CLOSE`；
   `repeat` body 保留单独路径，确保 condition 仍可见本轮 local。该根修复同时对齐 for-body
   cleanup、函数隐式 `RETURN` 前的 `CLOSE` 与相关 line scheduling；
6. `official/constructs.lua` 的剩余差异来自 parenthesized immediate 被过早物化，阻断固定
   C++ constant folding。括号现仅强制 single value，不再把 immediate/constant 提前装入寄存器，
   `2^9`、`2^5`、`2^2` 的精确 5 条 instruction 回归已通过；
7. fresh `target/compatibility/bytecode-structured-bounded-48-final/report.json` 为 48/48 passed、
   0 difference、0 infrastructure failure，共 393 个 Proto、14,595 条 instruction、2,772 个
   constant，覆盖 36/38 opcode，即固定 C++ 的 36/36 frontend-produced opcode；当前 workspace
   枚举 903 项测试；
8. pinned `target/compatibility/m2-structured-48-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、20,523 ms。fresh M1 重验
   `target/compatibility/m2-structured-48-final-m1-pinned/gate.json` 的静态合同、fmt、workspace
   Clippy、Debug/Release、warning-free rustdoc 与 raw-byte differential 全绿；唯一 hard failure
   仍是本机缺少 `cargo-audit`，用时 64,483 ms；
9. 下一主任务继续扩大 48-case 之外的有效 structured corpus；88 个 non-official process
   差异、真实 VM trace unsupported 与 M1 的跨平台 sanitizer/public allocator 债务仍开放，
   M2.2/M2.3 和总计划均未完成。

#### 16.3.38 已完成本地切片：60-case 官方压力语料与 literal/table-call parity

1. 新增 `tests/compatibility/bytecode-structured-bounded-60.json`，在 48-case 基础上加入
   官方 `api/attrib/big/code/db/errors/events/literals/math/nextvar/strings/vararg.lua`。12 个候选
   均先通过固定 C++ 与 Rust 独立可编译性筛查；首次 structured 结果为 57/60、3 项 semantic
   failure、0 infrastructure failure，失败集中在 `api.lua`、`attrib.lua`、`code.lua`；
2. `official/api.lua:393` 的普通 table array call fields 暴露临时寄存器泄漏：字段值调用物化到
   目标 slot 后，固定 C++ 将 free register 精确收回到 `targetReg + 1`，Rust 原来只保证下界，
   使第二、第三个调用 frame 右移。table emitter 现使用精确 `set_freereg`，并以三个 member-call
   字段的完整 19 条 root instruction 锁定；
3. `official/code.lua:78-81` 与 `official/attrib.lua:285-287` 共享另一个根因：固定 C++ 对
   `not not <literal>` 直接按 Lua truthiness 折叠为 boolean immediate，支持 nil、boolean、number、
   string 与括号 literal。Rust 已移植同一受限折叠合同，并以五类 literal 的精确 7 条 root
   instruction 锁定；不对任意有副作用或运行时表达式做扩大折叠；
4. 修复后 `api/attrib/code` 三个失败单例均为 1/1 passed；编译器精确 parity suite 当前
   33/33 passed，fresh `cargo test --workspace` 全绿，workspace 枚举 905 项测试；
5. fresh `target/compatibility/bytecode-structured-bounded-60-final/report.json` 为 60/60 passed、
   0 difference、0 infrastructure failure，共 596 个 Proto、32,583 条 instruction、5,493 个
   constant，覆盖 36/38 opcode，即固定 C++ 的 36/36 frontend-produced opcode；
6. pinned `target/compatibility/m2-structured-60-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、20,730 ms。fresh M1 重验
   `target/compatibility/m2-structured-60-final-m1-pinned/gate.json` 的静态合同、fmt、workspace
   Clippy、Debug/Release、warning-free rustdoc 与 raw-byte differential 全绿；唯一 hard failure
   仍是本机缺少 `cargo-audit`，用时 62,948 ms；
7. 下一主任务继续扩大 60-case 之外的有效 structured corpus；88 个 non-official process
   差异、真实 VM trace unsupported 与 M1 的跨平台 sanitizer/public allocator 债务仍开放，
   M2.2/M2.3 和总计划均未完成。

#### 16.3.39 已完成本地切片：72-case 全 vendored official structured coverage

1. 新增 `tests/compatibility/bytecode-structured-bounded-72.json`，在 60-case 基础上纳入剩余
   官方 `all/checktable/files/gc/main/pm/sort/verybig.lua`，以及 loadfile/dofile workflow、
   detailed setfenv、coroutine errors/status 四个复杂补充文件；12 个候选在固定 C++ 与 Rust
   两端均先通过独立可编译性筛查；
2. initial 与正式重跑均为 72/72 passed、0 difference、0 infrastructure failure；新增文件
   没有暴露新的 Proto/PC 差异，因此本切片不制造代码修复项。72-case 已纳入当前 vendored
   official suite 的全部 24 个 Lua 文件；
3. fresh `target/compatibility/bytecode-structured-bounded-72-final/report.json` 共覆盖 676 个
   Proto、40,137 条 instruction、6,669 个 constant 和 36/38 opcode，即固定 C++ 的 36/36
   frontend-produced opcode；
4. pinned `target/compatibility/m2-structured-72-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、21,569 ms；代码自 60-case
   修复后未变化，同一工作树的 fresh M1 证据仍为
   `target/compatibility/m2-structured-60-final-m1-pinned/gate.json`：静态合同、fmt、workspace
   Clippy、Debug/Release、warning-free rustdoc 与 raw-byte differential 全绿，唯一 hard failure
   是本机缺少 `cargo-audit`，用时 62,948 ms；
5. 下一主任务继续扩大 72-case 之外的 non-official structured corpus；88 个 process 差异、
   真实 VM trace unsupported 与 M1 的跨平台 sanitizer/public allocator 债务仍开放，
   M2.2/M2.3 和总计划均未完成。

#### 16.3.40 已完成本地切片：131/131 全量有效源码与 bytecode CLI fail-closed

1. 先在 72-case 之外加入 byte chunk source、VM limitation、runtime arg/global、GC、IO、env 与
   coroutine 多值等 12 个高价值 non-official 文件，84/84 初次即为零差异；随后停止机械分批，
   直接盘点 `tests/lua` 的全部 132 个 Lua 文件；
2. 双端独立可编译性筛查确认 131 个有效源码均由固定 C++ 与 Rust 成功编译；唯一例外是刻意
   非法的 `basic/syntax_error.lua`。新增
   `tests/compatibility/bytecode-structured-valid-131.json`，并用 inventory exact-set 校验保证它
   等于当前仓库全部 Lua 文件减去该唯一负例，不允许 silent omission；
3. 筛查同时发现 Rust `lua_bytecode` 对语法错误打印 stderr 后仍返回 0，而固定 C++ 返回非零。
   CLI `main` 现使用 `ExitCode`：成功返回 0、编译/IO/序列化错误返回 1、缺少参数返回 2；新增
   真实子进程回归验证语法错误无 stdout、stderr 有诊断且状态非零，实际
   `basic/syntax_error.lua --format=json` 返回 1；
4. initial 与正式全量比较均为 131/131 passed、0 difference、0 infrastructure failure。
   fresh `target/compatibility/bytecode-structured-valid-131-final/report.json` 覆盖 753 个 Proto、
   42,912 条 instruction、7,427 个 constant 和 36/38 opcode，即固定 C++ 的 36/36
   frontend-produced opcode；当前 workspace 枚举 906 项测试；
5. pinned `target/compatibility/m2-structured-valid-131-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、20,466 ms。fresh M1 重验
   `target/compatibility/m2-structured-valid-131-final-m1-pinned/gate.json` 的静态合同、fmt、
   workspace Clippy、Debug/Release、warning-free rustdoc 与 raw-byte differential 全绿；唯一
   hard failure 仍是本机缺少 `cargo-audit`，用时 30,316 ms；
6. 当前仓库的有效 structured corpus 扩展入口已耗尽，但这不自动关闭所有 M2.1/M2.2/M2.3
   极限与错误合同。下一主任务转向 compiler error/limit parity、88 个 non-official process
   差异与真实 VM trace；M1 的跨平台 sanitizer/public allocator 债务和总计划仍未完成。

#### 16.3.41 已完成本地切片：错误 chunk-id、source/line 与 CLI envelope

1. 从 fresh non-official 的唯一语法负例开始逐字节对照固定 C++：Rust 原先输出
   `@path:7: ...\n`，C++ 输出 `lua_app.exe: path:7: ...\r\n`。`lua_app` 现统一提取
   `argv[0]` basename、写入 `progname: message`，并只在 host diagnostic 边界使用平台文本 EOL；
2. `lua_vm::execute::chunk_name_for_diagnostic` 现集中实现 C++ `makeLuaChunkId` 合同：
   `@file -> file`、`=literal -> literal`、普通 source 使用 `[string "..."]` 摘要。compiler、
   runtime、base/package loader 和 CLI 共用该 renderer，内部 Proto 仍保留原始 source 标识；
3. CALL 非函数分支原先绕过 `runtime_error_at`，导致 `load` reader 中的 nil-call 丢失
   `(load):2`。该分支现从活动 Proto/PC 构造 location；
   `integration.env-metatable-loader` 的 stderr 已与固定 C++ 原始 bytes 完全一致；
4. 新增 2 个 chunk-id 单元测试和 2 个真实 `lua_app` 子进程精确测试。fresh
   `cargo test --workspace` 全绿，workspace 枚举 910 项；fmt、all-targets Clippy 与
   `git diff --check` 通过（仅既有 Windows EOL 提示）；
5. fresh `target/compatibility/m2-error-envelope-final-m0-pinned/non-official.json` 为
   selected 101、executed 92、helper 9、raw match 5、difference 87、runner error/timeout 0。
   `basic.syntax-error` 从 difference 变为 raw match；`stdlib.coroutine-errors` 的 `@` 泄漏和
   `integration.env-metatable-loader` 的 stderr provenance 均关闭，但两者仍受 stdout EOL
   债务影响。分析性路径+EOL 口径为 compared-channel 81/92、complete-channel 79/92；
6. pinned `target/compatibility/m2-error-envelope-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、20,248 ms。下一主任务转向
   M2.16 Windows host text-mode stdout/文件副作用，再继续 M2.1 compiler limits 与
   M2.7/M2.16 真实 VM trace；`tables.metamethod-arith` 的 arithmetic error location 仍开放。

#### 16.3.42 已完成本地切片：Windows CRT text-mode stdout/stderr 与文件副作用

1. 对固定 C++ 做 raw-byte 探针，确认 Windows CRT text output 对每个 `LF` 都插入 `CR`，
   包括已有 `CRLF -> CRCRLF`；`stdout`、`stderr` 和普通文本文件一致，`"wb"` 保持原字节。
   高位字节与 NUL 不经过 UTF-8 重编码；文本输入另确认 `CRLF -> LF` 与 Ctrl-Z EOF；
2. 新增集中 `lua_stdlib::host_io` 边界。base `print`、`io.write`、`io.stderr:write`、CLI
   usage/version/REPL/result/error 统一走 byte-oriented host text writer。文件状态保留原始 `b`
   mode；Windows 文本文件使用逻辑缓冲、读取时解码、落盘时转换，二进制/direct-write 路径保持 raw；
3. 同一探针的 Rust/C++ `stdout_lf`、`stdout_crlf`、`stderr_lf`、高位字节 `print`，以及
   `"w"`/`"wb"` 文件 hex 全部逐字节相等。新增 2 个 helper 单元测试、2 个真实进程测试，
   并把 3 个原先硬编码 Unix LF 的 coroutine 子进程断言改为显式 host text 合同；
4. fresh `cargo test --workspace` 全绿，workspace 枚举 914 项；fmt、all-targets Clippy 与
   `git diff --check` 通过（仅既有 Windows EOL 提示）。全量 non-official 从 5 status match /
   87 difference 提升为 80/12；manifest normalization 后全通道一致为 81/92，直接 raw
   stream/exit 一致为 78/92。三项差额是 fixture 已显式登记的 executable identity；
   `integration.env-metatable-loader` 两端 raw 完全一致但都以 1 退出，违反 manifest
   `expected_exit=0`。其余 11 个 normalization 后仍存在的实现间差异全部只在 stdout；
   stderr 与已登记文件副作用均为 92/92 一致；
5. 剩余 12 个 status debt 已收敛为：userdata `print` 2、GC count 7、IO text seek/`popen` 1、
   number formatting + arithmetic error location 1、双方共同 expected-exit failure 1。
   `stdlib.iolib` 的三个文本文件副作用已与 C++ exact-byte 相等，开放差异是 text `seek/tell`
   物理位置与缺失 `io.popen`，不再是落盘 EOL；
6. pinned `target/compatibility/m2-text-mode-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、20,320 ms。下一主任务转向
   M2.1 compiler error/limit parity，再推进 M2.7 真实 VM trace；M2.16 的 text seek/tell、
   `io.popen`、成熟 REPL 与剩余 CLI 合同继续保持开放。

#### 16.3.43 已完成本地切片：M2.1 compiler limits 与结构化 CodegenError

1. 固定 C++ inventory 确认 parser 合同为 expression recursion 92、block recursion 80、
   right-associative recursion 200、locals 200、upvalues 60；Rust 已有相同常量，但
   `jump::fix_jump` 对超长控制结构直接 panic，匿名函数表达式又会对 child codegen error
   `.expect(...)`。两项均可由普通 Lua 源码触发并以 Rust 101 / C++ 1 退出；
2. `CodeGenerator` 现记录首个结构化 lowering error，任何 partial Proto 都不能越过
   `generate`。子函数编译在错误路径也固定恢复 builder/register/local/upvalue/block/jump/
   line/liveness 状态；匿名 child 非法 lvalue 与畸形 method-call AST 均返回固定 C++ 的消息，
   不再 unwind。publication 回归确认错误后 temporary roots 为 0、partial graph 可完整 sweep；
3. 70,000-assignment 的 `if/while/repeat/numeric-for/generic-for` 控制流矩阵均与固定 C++ raw
   一致；典型 `if` 从 Rust panic 101 修为双方退出 1 且 stderr 精确为
   `lua_app.exe: loadstring: control structure too long\r\n`。顶层与匿名函数内非法 lvalue
   也都精确匹配 `Expression is not a valid lvalue`；
4. 新增 parser limit 回归，覆盖 90/150 parentheses、79/80 nested blocks、400 concat、400
   nested calls、200/201 locals、60/61 upvalues 与 malformed method token/category。双端 8-case
   CLI artifact 全部 raw match；300 参数越过 8-bit register 字段的 structured bytecode 为 1/1
   零差异，262,145 个不同常量越过 `MAXARG_Bx` 的 1.72 MiB probe 双方都退出 0。后两项锁定
   固定 C++ 的截断合同，不擅自引入 oracle 不存在的新错误；
5. fresh `cargo fmt --all -- --check`、workspace all-targets Clippy `-D warnings` 与
   `cargo test --workspace` 全绿，workspace 枚举 925 项。fresh full-valid structured bytecode
   `target/compatibility/m2-compiler-limits-bytecode-131-final/report.json` 为 131/131 passed、
   0 semantic/infrastructure failure；
6. pinned `target/compatibility/m2-compiler-limits-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、21,779 ms；non-official 保持
   status 80/12、manifest-normalized 81/92、direct raw 78/92，说明本切片没有扩大
   normalization 或改写无关语义。M2.1 标记 `completed-local`；下一主任务转向 M2.7 真实 VM
   trace，再处理 M2.16 text seek/tell、`io.popen` 与剩余 CLI 合同。

#### 16.3.44 已完成本地切片：M2.7 真实 VM Snapshot/Basic 12-case parity

1. Rust Runtime 新增可关闭/flush/fail-closed 的 JSONL instruction trace；`lua_app --trace file`
   在每条真实指令执行前输出 `pc/opcode/operands/source/line/callDepth` 与完整寄存器快照，
   `--trace-diff` 继续显式报 unsupported，不伪装成已实现；
2. 固定 C++ CLI 的 trace sink 绑定错误 singleton、真实运行产生 0 event。仓库因此在不修改
   pinned checkout 的前提下新增只读 `lua_vm_trace_evidence` adapter，链接安装后的固定 C++
   `lua_core`，并由 `build_cpp_oracle.ps1` 同步构建；
3. `compare_vm_trace.ps1` 现支持 `Snapshot|Diff` 与 `Basic|Complete` 证据画像，缺少真实
   event kind/coverage 会 fail closed，不能用空集合使 Complete profile 假绿。runner 自测同时
   覆盖 synthetic Complete、真实 C++ Snapshot producer 与真实 Rust Snapshot producer；
4. 新增固定 12-case manifest，覆盖 arithmetic、break/numeric-for、nested-if、closure、vararg、
   multiret、table-constructor multiret、基础调用和 `setmetatable`。fresh 跨实现
   Snapshot/Basic artifact 为
   `target/compatibility/m2-real-trace-observational-final-m0-pinned/vm-trace-snapshot-basic/report.json`，
   12/12 passed、0 difference、0 infrastructure failure；
5. C++ TEST/TESTSET 内联 JMP、numeric-FOR 终止探测与 native frame 死寄存器存在内部表示差异。
   Rust trace writer 使用按 state/slot/instruction-write 失效的观测投影来对齐这些逻辑值，
   不扩展 deferred continuation frame、不改 VM 栈；`alien_signals` 与 1000 层 resume/wrap
   两项深链回归均通过；
6. fresh `cargo fmt --all -- --check`、workspace all-targets Clippy `-D warnings` 与
   `cargo test --workspace` 全绿，workspace 枚举 926 项。完整 pinned M0 artifact
   `target/compatibility/m2-real-trace-observational-final-m0-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、2 debts、25,188 ms；non-official 保持
   status 80/12；
7. M2.7 仅完成 Snapshot/Basic 第一切片，尚未关闭。下一主任务是 `--trace-diff`、stack top、
   changed-register、upvalue open/close、call/return/yield/resume 与 error value/category 事件，
   随后处理 M2.16 text seek/tell、`io.popen` 与剩余 CLI 合同。

#### 16.3.45 已完成本地切片：M2.7 真实 Diff/Basic、stackTop 与 changedRegisters

1. Rust `lua_app --trace-diff file` 已改为真实 producer：每条指令保存执行前窗口，在后状态
   可观察时输出 JSONL `changedRegisters`，字段包括 slot/name、old/new 与 oldType/newType；
   最终 RETURN 会显式 flush，事件序号按实际输出顺序连续；
2. 固定 C++ evidence adapter 同时支持 `--trace`/`--trace-diff`，并从实际
   EngineContext-owned `LuaState` 输出 `stackTop`。`compare_vm_trace.ps1` 的 C++ Diff lane
   也改用该 adapter，避开固定 CLI 的 legacy singleton sink 缺陷；
3. Rust trace 层只读投影固定 C++ 的 frame-top、vararg precall、固定/开放多返回与 RETURN
   回写布局，保持 Rust VM stack、CallInfo 与 deferred continuation 不变。Snapshot 和 Diff
   共用 per-state/frame top 模型，新增字段没有破坏原 Snapshot 合同；
4. 同一固定 12-case manifest 的 fresh 跨实现结果为 Snapshot/Basic 12/12、Diff/Basic 12/12，
   两者均为 0 difference、0 infrastructure failure。Diff artifact 位于
   `target/compatibility/m2-real-trace-diff-pinned-final/vm-trace-diff-basic/report.json`；
5. runner 自检新增 C++/Rust Diff producer 自比较，并用真实跨实现 Complete/Diff 验证缺少
   upvalue/control/yield/resume/error 时精确留下 3 个 `missing-evidence`，退出 1，防止双方
   同时缺字段造成假绿；
6. fresh fmt、workspace all-targets Clippy `-D warnings` 与 `cargo test --workspace` 全绿，
   workspace 枚举 927 项。完整 pinned M0 artifact
   `target/compatibility/m2-real-trace-diff-pinned-final/gate.json` 为 `passed=true`、
   `fullGatePassed=true`、0 hard failure、2 debts、35,515 ms；Snapshot 与 Diff 双 12-case
   均为硬门；
7. M2.7 仍为 partial：下一主任务缩小为 upvalue open/close、call/return/yield/resume 与
   error value/category lifecycle 事件；随后处理 M2.16 text seek/tell、`io.popen` 与剩余
   CLI 合同。

#### 16.3.46 已完成本地切片：M2.7 真实 CALL/RETURN 与顶层 error evidence

1. Rust trace producer 已在真实 `CALL`/`RETURN` opcode 边界输出控制事件：Snapshot 保持
   instruction→control 顺序，Diff 保持 control→post-instruction diff 顺序；funcName、source、
   line、callDepth 与固定 C++ `emitCallTrace`/`emitReturnTrace` 合同逐事件一致；
2. checkout 外 C++ evidence adapter 已从 instruction-only sink 升级为完整转发固定 VM 的
   Call/Return event。固定 C++ 虽定义 Error 类型却没有 opcode producer，因此两端都在顶层
   protected execution boundary 输出未捕获 error，字段固定为 message/errorValue、
   `errorCategory="runtime"` 与全局连续 seq；
3. Rust `error(message)` 的顶层 frame 过去因没有 Function stack slot 而丢失 source:line。
   `error_location_prefix` 现回退到活动 CallInfo Proto，专项错误夹具的 stderr、exit=1 与
   errorValue 和固定 C++ 完全一致；新增 CLI 可观察回归后该测试集为 18/18；
4. `compare_vm_trace.ps1` 的 Basic 画像现显式投影 instruction events 并重排投影 seq；原始
   完整事件仍用于全局序号和 coverage，Complete 继续比较全部事件。runner 同时允许双方已完成
   且 exit code 相同的非零执行进入语义比较，退出码不同时仍 fail closed；
5. `tests/trace_fixtures/call_return_error.lua` 的 Complete/Diff 跨实现报告观察到
   call/instruction/return/error 四类事件，`callReturnYieldResume=true`、
   `errorValueCategory=true`，无事件、stdout、stderr 或 exit 差异；唯一 difference 精确为
   `evidence.upvalueLifecycle`。runner/M0 自检已锁定“只缺这一项”，不再接受旧的三项泛化缺口；
6. fresh Snapshot/Basic 与 Diff/Basic 代表集仍分别 12/12，artifact 位于
   `target/parity/vm-trace-lifecycle-snapshot-basic-12/report.json` 与
   `target/parity/vm-trace-lifecycle-diff-basic-12/report.json`。fresh fmt、workspace
   all-targets Clippy `-D warnings`、`git diff --check` 与 `cargo test --workspace` 全绿，
   workspace 枚举 928 项；C++ adapter 继续通过 `/W4 /WX`；
7. 完整 pinned M0 artifact
   `target/compatibility/m2-real-trace-lifecycle-pinned/gate.json` 为 `passed=true`、
   `fullGatePassed=true`、0 hard failure、2 debts、39,272 ms，双 12-case 仍为硬门；
8. M2.7 仍为 partial：当前 coverage 名 `callReturnYieldResume` 只证明至少存在一类控制事件，
   不能外推为 yield/resume 已实现。下一主任务固定为 upvalue open/close 与显式 yield/resume
   事件，再处理 M2.16 text seek/tell、`io.popen` 与剩余 CLI 合同。

#### 16.3.47 已完成本地切片：M2.7 Complete 核心 lifecycle 与稳定 trace state identity

1. `tests/compatibility/vm-trace-complete-lifecycle.json` 现固定四个真实负例：顶层
   CALL/RETURN/error、首次 resume 的 CALL/RETURN 控制顺序、nested protected error，以及
   同 Proto 双 coroutine upvalue；每例双端都以 1 退出且无 timeout/start error；
2. Rust 与固定 C++ adapter 均输出 instruction/call/return/yield/resume/upvalue-open/
   upvalue-close/error 八类 Complete evidence。Diff/Complete 和 Snapshot/Complete 各为
   4/4 passed、0 difference、0 infrastructure/semantic failure；runner 与 M0 还锁定 exact
   requiredEvidence、profile/mode/purpose、case 数、双端 coverage 和退出证据，不能用空列表假绿；
3. Rust 修复了 coroutine error 帧保留、protected unwind、native TAILCALL close/RETURN、
   pcall/xpcall scratch 布局、protected yield barrier、Automatic-GC continuation 和 callback
   opcode 屏障；fixed C++ adapter 修复了 active coroutine state、nested/wrap/Normal activation、
   upvalue membership、首次控制事件顺序及 Diff/Snapshot stack-top 投影；
4. Rust trace 的所有 state-scoped key 已从裸地址迁移到带 RuntimeId/slot/generation 的
   `StateHandle`。StateArena 在 generation advance 前发布成功的 shutdown upvalue-close，
   flush lifecycle 并 purge pending/projection/suppression cache；仓库回归证明重复六次
   create→suspend→collect 不会让旧 state 抑制新 upvalue-open；
5. metamethod/debug callback barrier 已消除“外层 opcode Diff 早于 callback 指令”的反序；
   callback RETURN 现分离事件 result top 与 caller projection，debug-hook 恢复保持普通 Lua
   CALL depth，并保留 hook 对 caller open-upvalue 的写入。两例在 Diff/Snapshot 四条 lane
   均零差异且已进入 parity runner。广域 M2.7 仍保持 partial：固定 C++ core 没有
   state/thread 析构回调或稳定 generation，adapter 虽以
   `(LuaState*, Thread*) + continuation` 安全淘汰常见复用缓存，却不能证明所有关联地址同时复用
   时的 GC retirement close；彻底关闭需要 core oracle hook，不能依赖 dangling pointer；
6. fresh `cargo check --workspace`、`cargo test --workspace`、all-targets Clippy
   `-D warnings`、warning-free rustdoc、fmt 与 `git diff --check` 全绿，workspace 枚举 941 项；
   C++ evidence adapter 通过 `/W4 /WX` Release；
7. 完整 pinned M0 artifact
   `target/compatibility/m2-callback-final-pinned/gate.json` 为 `passed=true`、
   `fullGatePassed=true`、0 hard failure、1 debt、48,798 ms；Snapshot/Basic 与 Diff/Basic
   均为 12/12，Complete/Diff 核心 4-case 已进入 fail-closed 自检。唯一 gate debt 是
   `non-official-semantic-differences`（80 match / 12 differences / 9 helpers skipped）；
8. 下一主任务保持 M2.7：关闭 C++ trace-state generation/GC-retirement oracle 缺口，再推进
   M2.16 text seek/tell、`io.popen` 与剩余 CLI 合同。

#### 16.3.48 已完成本地切片：M2.7 GC retirement 与 M2.12 IO seek/popen

1. 固定 C++ trace evidence adapter 现为独立 `EngineContext` 配置等价 malloc/realloc/free
   allocator。sink 观察到的 `LuaState*` 在析构完成、开放 upvalue 已关闭后收到精确
   deallocation 通知；通知只登记退役，Snapshot 在下一 instruction 前、Diff 在当前
   post-instruction 后发布 close，并清理旧 state identity/projection/upvalue cache；不再依赖
   `(LuaState*, Thread*)` 地址复用猜测；
2. 新增 `test_vm_trace_state_retirement.ps1` 与 `coroutine_state_retirement.lua`。C++、Rust
   producer 均必须独立证明 3 次 captured open、前两次 GC 的 2 次 close、总计 6 open/5 close、
   双端 exit=1 及 Complete 八类事件；两份报告进入 parity runner 与 M0 artifact hard validation；
3. M2.7 现为 `completed-local`：Basic Snapshot/Diff 双 12-case、Complete 核心双模式 4-case、
   callback 双模式 2-case 与双 producer retirement 自检全部通过；这不外推为 M2.3 全域 trace
   corpus 完成；
4. Windows text path file 的 `seek/tell` 现显式映射逻辑 buffer cursor 与 CRT physical opaque
   offset；`stdlib.iolib` 文件长度由 Rust 32 对齐为 C++ 35，binary 与 tmpfile offset 保持原合同，
   CRLF/CRCRLF、`set/cur/end` 与负 offset 有定向测试；
5. `io.popen` 已按当前 unrestricted 默认注册并实现。Windows 使用 `cmd /C`、POSIX 使用
   `sh -c`，覆盖 read/write pipe、host text EOL、成功与非零 child close tuple、显式 close、
   Lua `__gc`/Runtime resource drain；未来 M4 sandbox capability 仍需收敛配置策略；
6. 完整 non-official 从 80/12 推进到 81/11；`stdlib.iolib` 1/1 完全 match，三个文件副作用仍
   exact-byte 相等，runner error/timeout 为 0；
7. fresh `cargo check --workspace`、`cargo test --workspace`、all-targets Clippy `-D warnings`、
   warning-free rustdoc、fmt 与 `git diff --check` 全绿，workspace 枚举 947 项；C++ adapter
   `/W4 /WX` Release 通过；
8. fresh pinned M0 artifact `target/compatibility/m2-io-final-pinned/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、1 debt、51,494 ms；唯一 debt 仍为
   `non-official-semantic-differences`（81 match / 11 differences / 9 helpers skipped）。下一主任务
   转为 M2.16 剩余 CLI argument/stdin/hash-bang/source-name 合同。

#### 16.3.49 已完成本地切片：M2.16 CLI、stdin/source-name 与 REPL 错误恢复

1. `lua_app` 的缺失脚本错误已从平台本地化的 `std::io::Error` 收敛为固定 C++ 合同
   `cannot open file: <path>`；`-v`、help 与缺失 `--trace-diff` 参数的文案也与 pinned C++ 对齐；
2. REPL 每次提示都在受管 Runtime turn 内读取 `_PROMPT/_PROMPT2`。交互语法错误使用
   `stdin:<line>`，运行时错误使用 `(repl):<line>`，错误写入 stderr 后清理当前输入并继续会话，
   不再以进程 exit 1 提前结束；
3. 新增 `tools/test_cli_contract.ps1`，以固定期望和跨语言原始进程证据双重校验 18 个 case：
   参数、stdin、`-e/-l/-i/--`、script args、动作顺序、hash-bang、文件错误、source name、
   stdout/stderr/exit 与交互错误恢复。公共 process runner 新增可选 stdin bytes，旧调用默认不变；
4. CLI runner 已进入 `test_parity_runners.ps1` 和 M0 hard artifact validation。报告固定
   `selected=18`、`passed=18`、0 failure/infra；binary chunk execution 是唯一 deferred，明确
   路由 M3 serializer/deserializer，不以源码 parser 近似；
5. fresh `cargo check --workspace`、`cargo test --workspace`、all-targets Clippy `-D warnings`、
   warning-free rustdoc、fmt 与 `git diff --check` 全绿，workspace 精确枚举 951 项；
6. fresh pinned M0 artifact `target/compatibility/m216-cli-final-pinned-v2/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、1 debt、51,021 ms；non-official 保持
   81 match / 11 differences / 9 helpers skipped，CLI 18/18 在重建后的双实现上复验；
7. M2.16 标记 `completed-local`，不外推为 M2.17/M2.18 或 M3 完成。下一主任务转为 M2.17
   剩余多行/不完整输入、stack reset、history/meta/completion 与 Ctrl-C 合同。

#### 16.3.50 已完成本地切片：M2.17 完整 REPL 合同与门禁接入

1. `lua_app` 的无参数模式现提供固定 C++ banner、Lua 5.1 多行/不完整输入、自动表达式与
   前导 `=`、错误后继续、每命令 stack reset、`exit/quit` 与精确 `Goodbye!`；`-i` 继续使用
   quiet REPL，避免改变已有 CLI action 顺序；
2. 新增 `.help`、`.bytecode`、`.ast`、`.gc` 与 `.lua_history`。bytecode/AST 对表达式与 chunk
   使用 pinned C++ 的展示合同，GC meta command 调用真实 Runtime full collection；历史文件
   读写失败保持非致命；
3. completion 覆盖 meta/global/table-field 与歧义候选。Windows 真实终端通过 `_getch`
   处理 Tab、Backspace、Ctrl-C 和 Ctrl-D/Z；重定向 stdin 保持 byte-oriented、无 BOM 的可重放行为；
4. 新增 `tools/test_repl_contract.ps1`，以隔离工作目录和历史文件对 8 个双实现 case 做固定期望与
   raw stdout/stderr/exit 双重校验。CLI 18/18 与 REPL 8/8 均进入 parity self-test 和 M0
   fail-closed artifact validation；
5. fresh `cargo test --workspace` 精确枚举 956 项；`cargo doc --workspace --no-deps` 配合
   `RUSTDOCFLAGS=-D warnings`、all-targets Clippy `-D warnings`、fmt 与 `git diff --check` 全绿；
6. fresh pinned M0 artifact `target/compatibility/m217-repl-m0-final/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、1 debt、59,706 ms；23 个步骤全部通过，
   non-official 保持 81 match / 11 differences / 9 helpers skipped；
7. M2.17 标记 `completed-local`，不外推为 M2.18 或完整 Phase 2 完成。下一主任务转为
   M2.18 bytecode viewer 的错误参数、严格 format、转义与 full/cfg/diff 模式合同。

#### 16.3.51 已完成本地切片：M2.18 bytecode viewer 全模式与 byte-safe 输出

1. `lua_bytecode` 参数解析已与固定 C++ 对齐：compact、`full/--full`、`--cfg`、双文件
   `--diff`，未知选项/数量/冲突以 exit 1 和固定 usage fail closed；文件或编译异常以 exit 3，
   失败时不留下部分 stdout；
2. text printer 现输出完整 Proto metadata、RK/constant/sub-Proto 注释和递归 child Proto；
   full、Mermaid CFG 与 side-by-side diff 均逐字节复刻 pinned C++。string constant 的引号、
   反斜杠、换行、tab 与 0x80–0xff 原始 byte 均有回归，text/diff 不做 UTF-8 重编码；
3. 既有 `--format=json` 保留为 Rust 结构化证据扩展，继续使用 serde_json schema 2 与
   byte envelope；未知 format 和 JSON 与 full/cfg/diff 的非法组合会被严格拒绝；
4. 新增 `tools/test_bytecode_tool_contract.ps1`：12 个跨语言 raw stdout/stderr/exit case 加
   3 个 Rust JSON/strict-format case，共 15/15；报告进入 parity self-test 与 M0 artifact
   fail-closed 强校验；
5. fresh `cargo test --workspace` 精确枚举 958 项；workspace check/tests、all-targets Clippy
   `-D warnings`、warning-free rustdoc、fmt、PowerShell AST 与 `git diff --check` 全绿；
6. fresh pinned M0 artifact `target/compatibility/m218-bytecode-m0-final/gate.json` 为
   `passed=true`、`fullGatePassed=true`、0 hard failure、1 debt、80,581 ms；23 个步骤全部通过，
   bytecode tool contract 15/15，non-official 保持 81 match / 11 differences / 9 helpers skipped；
7. M2.18 标记 `completed-local`，不外推为 binary chunk、M2 全域或 Phase 2 完成。下一主任务
   转为 M2.9 `tables.metamethod-arith` 的 arithmetic error object/source-line provenance。
