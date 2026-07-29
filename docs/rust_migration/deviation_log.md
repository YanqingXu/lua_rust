---
status: living
last_updated: 2026-07-29
applies_to: Lua 5.1.5 compatibility, lua_cpp project extensions, and runtime safety
oracle_cpp_commit: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Compatibility Deviation Log

本日志登记 `lua_rust` 与双 oracle（官方 Lua 5.1.5 和固定提交的
`lua_cpp`）之间已确认的差异、近似实现和缺失能力。模块存在或项目内测试通过，
不代表差异已经关闭；只有对应 oracle 测试通过，或差异被明确批准，状态才能改变。

## 状态定义

| 状态 | 含义 |
|---|---|
| `approved` | 维护目标有意不同于 stock Lua；差分测试必须显式引用本 ID。 |
| `open` | 已确认差异仍存在，不能作为兼容完成项。 |
| `remediation-pending-verification` | 修复已在当前工作树中实现，但尚未通过完整验收门。 |
| `remediation-verified-local` | 修复已通过本地完整验收，尚待远程 CI 首次运行与合入。 |
| `decided-in-progress` | 目标行为已在当前里程碑决策，代码迁移与差分验收仍在进行。 |
| `resolved` | 修复及其 oracle 验证均已合入；条目保留用于审计。 |

## 登记项

### NOTE-001: 项目扩展 `_VERSION` 值

- **日期：** 2026-07-26
- **范围：** base library 全局变量 `_VERSION`。
- **Rust 位置：** `crates/lua_stdlib/src/base.rs`；
  `crates/lua_app/tests/observable_io.rs`。
- **当前/目标行为：** `_VERSION` 为
  `Lua 5.1 (C core prototype)`。
- **Oracle：**
  - 固定的 `lua_cpp@87c15e6` 在 `src/lib/baselib.cpp` 注册相同项目扩展值；
  - stock Lua 5.1 的 `_VERSION` 为 `Lua 5.1`。
- **测试与任务：** `tests/compatibility/lua51-version-probe.lua`；
  `crates/lua_app/tests/observable_io.rs`；M0.5、M2.8。
- **影响：** 对 `_VERSION` 做精确字符串比较的 stock Lua 脚本会看到有意差异。
- **处置状态：** `approved`。项目兼容优先级选择 `lua_cpp` 值；官方 Lua
  differential 必须把该差异标为 `NOTE-001`，不得归一化或静默忽略。

### NOTE-002: GC 增量与 allocator 可观察行为尚未闭环

- **日期：** 2026-07-29
- **范围：** `collectgarbage`、`gcinfo`、增量 step、弱表、终结器、内存计账和
  collector shutdown。
- **Rust 位置：** `crates/lua_stdlib/src/base.rs`；
  `crates/lua_vm/src/runtime/full_collection.rs`；
  `crates/lua_vm/src/runtime/incremental_collection.rs`；
  `crates/lua_core/src/gc/`。
- **当前行为：** Runtime safe-point full STW 已消费 canonical tracer，按
  mark→finalizer prepare/resurrection propagation→weak reconciliation→state
  prepass→sweep→protected callback 执行，对任何 root gap/foreign edge
  fail-closed。Lua `collectgarbage("collect")` 已调用真实回收，`gcinfo/count`
  读取 collector accounted bytes，weak v/k/kv、exactly-once、异常隔离、
  reentrant collect、resurrection/再次死亡和 close drain 均有回归。
  `collectgarbage("step")` 已使用 pause→propagate→atomic→sweep→finalize、
  Runtime 持久 object/StateHandle 双队列、debt/pause/stepmul 和有界 work
  unit；8-family production mutation barrier 与 active-allocation publication
  已接线。allocator live/peak 和 allocation-triggered automatic checkpoint
  尚未实现。
- **Oracle：** stock Lua 5.1 的可达性和 GC API；`lua_cpp@87c15e6` 的
  `tests/unit/gc/test_gc.cpp`、official suite GC probe 与 shutdown/lifecycle
  行为。
- **测试与任务：** `tests/lua/differential/gc-weak-value.lua`；
  full STW 的两轮全图、weak/finalizer/resurrection、typed Drop、
  state/upvalue prepass、stale handle、cross-collector/root-gap/phase 回归；
  M1.7–M1.13，当前重点为 M1.13。
- **当前最小证据：** M0 weak-value probe 在官方 Lua 与 C++ oracle 两条 lane
  均通过；Rust 另有 13 项新增 workspace 回归覆盖公开 full collection 和上述
  weak/finalizer 生命周期，另有 12 项增量/屏障/控制回归。它们不证明
  automatic collection 或 allocator live/peak 合同。
- **影响：** Lua 脚本已能显式触发真实 full/增量回收并观察 collector
  accounted bytes；但自动回收和 allocator 指标仍不能视为兼容，长生命周期
  未显式 step/collect 的运行路径仍可能积累对象。
- **处置状态：** `open`。full STW、weak/finalizer/resurrection、public
  `collectgarbage("collect"/"step")`、mutation barrier、实际 `gcinfo/count`
  和 close drain 子项已关闭；automatic integration、allocator 与完整
  shutdown 验收全部通过前，Phase 1 与 GC 相关的 Phase 3/4 能力保持 partial。

### NOTE-003: `string.dump` 不是 Lua 5.1 binary chunk

- **日期：** 2026-07-26
- **范围：** `string.dump`、`load`/`loadstring` 的 dump 回读和
  `lua_bytecode`。
- **Rust 位置：** `crates/lua_stdlib/src/string.rs`、
  `crates/lua_stdlib/src/base.rs`、`crates/lua_app/src/main.rs`。
- **当前行为：** M1.6 已删除 `DUMPS`/`SOURCES` thread-local registry 和
  `LuaRustDump` 回读旁路，不再长期保存未受 root 证明的 `GcRef<Proto>`。
  `string.dump` 在验证参数是 function 后，通过现有 C callback 负返回值合同
  抛出固定错误：
  `string.dump is unsupported until Lua 5.1 binary chunk serialization is implemented`。
  `load`、`loadstring` 与 CLI 文件入口只按源码解析，旧伪 dump 会得到语法
  错误而不会回退编译内嵌源码。
- **Oracle：** stock Lua 5.1 `lundump`/`ldump` 格式；固定
  `lua_cpp@87c15e6` 的 binary chunk 行为。
- **测试与任务：**
  `string_dump_reports_stable_unsupported_error`、
  `legacy_pseudo_dump_is_not_accepted_by_load_or_loadstring` 与
  `command_line_rejects_legacy_pseudo_dump_files` 固定 M1.6 的显式失败边界；
  M3.1–M3.3 仍负责真实 serializer/deserializer。
- **影响：** 悬垂 registry 风险和伪持久化假象已移除；依赖 `string.dump` 或
  binary chunk 加载的程序会明确失败，直到 M3 实现官方格式。
- **处置状态：** `mitigated-open`。M1.6 的生命周期风险已关闭；binary chunk
  功能差距在 M3 serializer/deserializer 和双 oracle 验收完成前保持 open。

### NOTE-004: 默认标准流从 memory file 迁移到宿主流

- **日期：** 2026-07-26
- **范围：** `io.stdin`、`io.stdout`、`io.stderr`、`io.read`、
  `io.write`、file handle `read/write/flush` 及进程重定向。
- **Rust 位置：** `crates/lua_stdlib/src/io.rs`；
  `crates/lua_app/tests/observable_io.rs`。
- **基线行为：** 提交 `6284135` 把三个默认流创建成 memory file，导致
  `io.write` 和 `io.stderr:write` 对父进程不可观察，stdin pipe 也不能作为
  默认输入。
- **当前工作树：** M0 修复已把默认 handle 连接到宿主 stdin/stdout/stderr，
  新增进程级 pipe/redirection 测试。本地 `rust_quality_gate.ps1 -SkipAudit`
  在 M0 收口时已通过 596 个 Debug/Release workspace tests；当时的 129-item
  manifest、101-case
  non-official runner 和 4-case 双-oracle differential 均通过基础设施门，
  其中两条 differential lane 都是 4/4。统一 M0 gate 为 `passed=true`、
  `hardFailures=0`。远程 CI 尚待首次运行。
- **Oracle：** stock Lua 5.1 CLI；`lua_cpp@87c15e6` 的进程级 stdout、
  stderr、stdin 和 exit-status 行为。
- **测试与任务：** `crates/lua_app/tests/observable_io.rs`；
  `tests/lua/differential/stderr.lua`；M0.4、M0.5、M2.8、M2.12。
- **影响：** 基线版本的 CLI 输出捕获、管道、重定向和 differential 会假绿或
  无输出。
- **处置状态：** `remediation-verified-local`。本地 process、fixture、
  双-oracle differential 和统一 M0 gate 已验证；只有远程 CI 首次运行通过
  且修复合入后，才能改为 `resolved`。

### NOTE-005: `package.loadlib` 明确不支持动态库

- **日期：** 2026-07-26
- **范围：** native module 搜索、加载、符号解析、调用、卸载和生命周期。
- **Rust 位置：** `crates/lua_stdlib/src/package.rs`。
- **当前行为：** `package.loadlib` 返回
  `dynamic libraries not supported: ...`；没有 `dlopen`/`LoadLibrary`
  后端，也没有 native module registry。
- **Oracle：** stock Lua 5.1 package loader；`lua_cpp@87c15e6` 的
  native module 与 public API 测试。
- **测试与任务：** 官方 `attrib.lua`/`api.lua` native cases；M2.15、
  M3.9。
- **影响：** C/C++ Lua 模块无法由 `lua_rust` 加载；相关官方用例为预期失败。
- **处置状态：** `open`。M3.9 及 module lifecycle 验收通过前不得标记支持。

### NOTE-006: OS、locale 与 time 使用平台无关近似

- **日期：** 2026-07-26
- **范围：** `os.clock`、`os.date`、`os.time`、`os.setlocale`、
  `os.execute`、`os.tmpname` 及错误 tuple。
- **Rust 位置：** `crates/lua_stdlib/src/os.rs`。
- **当前行为：** `os.clock` 返回进程内 wall-clock elapsed time，而非 C
  `clock()` 的进程 CPU 时间；date/time table 以自实现 UTC civil conversion
  处理，忽略本地时区和 DST；`!` 前缀未形成与 local-time 的差异；
  `setlocale` 只接受/报告 `C`；格式符、错误返回和平台 exit status 也未完整
  对齐。
- **Oracle：** stock Lua 5.1 在目标 OS/C runtime 上的行为；固定
  `lua_cpp@87c15e6` 的 OS library 测试。平台允许差异必须逐项记录，不能跨平台
  统一假设。
- **测试与任务：** `crates/lua_stdlib/tests/stdlib_integration_tests.rs` 现有
  C-locale/UTC 子集测试；M2.13、M5.1。
- **影响：** locale、时区、DST、CPU-time、格式化和 shell 状态相关脚本可能
  得到不同结果。
- **处置状态：** `open`。现有测试只覆盖项目内近似，不是兼容完成证据。

### NOTE-007: Lua 字符串尚未采用任意字节表示

- **日期：** 2026-07-26
- **范围：** `GcString`、StringPool、lexer/parser、string/io/package/os、
  文件与 CLI 输入、哈希、长度和 C API pointer+length。
- **Rust 位置：** `crates/lua_core/src/gc_string.rs`；
  `crates/lua_core/src/string_pool.rs`；`crates/lua_compiler/src/lexer.rs`；
  `crates/lua_stdlib/src/string.rs`；`crates/lua_stdlib/src/io.rs`。
- **当前行为：** ByteString/GcString/StringPool 已承载任意 bytes；生产 compiler、
  VM、stdlib、app、参数/错误/字段/name 构造强制走 owning StringPool。
  `Value::String` Eq/Hash 使用不可复用的 `GcRef` identity 而不解引用，内容语义
  经 collector/state-scoped bytes 完成。17-path 静态 inventory/gate 与
  duplicate/foreign/stale/NUL/high-byte/address-reuse 回归已锁定该边界。
- **Oracle：** stock Lua 5.1 字符串是任意 byte sequence；固定
  `lua_cpp@87c15e6` 的 GCString/hash/string library 与 C API
  pointer+length 行为。
- **测试与任务：** NUL、`0x00..0xff`、invalid UTF-8、UTF-8 原始字节、
  hash/intern/pointer+length corpus；M1.1–M1.3、M2.10。
- **影响：** 高位字节、无效 UTF-8、长度、切片、pattern、IO round-trip、
  dump 和未来 C API 可能不兼容或双重编码。
- **处置状态：** `mitigated-open`。核心表示与当前生产 identity/access 合同已
  本地闭合；真实 binary dump/load、未来 C API pointer+length 与全链路双-oracle
  验收完成前，整体字符串兼容状态仍为 partial。

### NOTE-008: Lua 5.1 C API/ABI 尚不存在

- **日期：** 2026-07-26
- **范围：** `lua.h`/`lauxlib.h`/`lualib.h`、state/stack/type/table/call/
  coroutine/GC API、allocator callback、static/shared library 和外部 consumer。
- **Rust 位置：** 当前 workspace 没有 `lua_capi` crate 或公开 C headers。
- **当前行为：** Rust 内部使用 `unsafe extern "C"` 函数指针承载宿主函数，
  但这不是 Lua C API，也没有稳定 ABI、opaque state handle 或 panic boundary。
- **Oracle：** stock Lua 5.1 headers/API；固定 `lua_cpp@87c15e6` 的
  public export contract、123 项机器合同和 C/C++ consumer。
- **测试与任务：** 目前无 Rust C consumer；M3.4–M3.8。
- **影响：** 不能把 `lua_rust` 作为嵌入式 Lua 5.1 库链接，也不能移植依赖
  Lua C API 的宿主或模块。
- **处置状态：** `open`。在 123/123、导出符号和独立 consumer 验收前，
  C API 状态为 not-started。

### NOTE-009: Runtime 与 coroutine 所有权未闭环

- **日期：** 2026-07-26
- **范围：** collector、StringPool、main/coroutine `LuaState`、registry、
  service pointers 和确定性 shutdown。
- **Rust 位置：** `crates/lua_vm/src/state/lua_state.rs`；
  `crates/lua_vm/src/runtime.rs`；
  `crates/lua_vm/src/runtime/root_trace.rs`；
  `crates/lua_core/src/upvalue.rs`；
  `crates/lua_stdlib/src/coroutine.rs`；
  `crates/lua_core/src/thread.rs`；
  `crates/lua_core/src/gc/collector.rs`。
- **当前行为：** pinned Runtime/StateArena 已独占 coroutine Box，Thread
  只保存受 runtime/slot/generation 校验的 handle；RuntimeId 不回绕且不可由
  safe raw integer 重建，generation `u64::MAX` 释放后永久退休。Runtime
  close/Drop 已执行 state→Thread→ordinary→fixed 的 Rust-owned 确定性销毁。
  coroutine resume/wrap 已改为 sealed runtime-native request：scoped
  mailbox 与独立 `VmExit::NativeRequest` 暂停 caller，deferred C frame 和
  Runtime activation stack 在释放 caller borrow 后驱动 target；caller
  `Running↔Normal`、yield/result/error transfer、protected `pcall` 与固定
  C++ 的 `A→B→A` `Normal` 祖先 continuation 二次执行已有回归。
  open Upvalue 已改为 `Open { owner: StateHandle, stack_index }`，由 owner
  state 维护非 intrusive、按索引降序且去重的集合；跨 state GET/SET 通过
  Runtime upvalue transfer turns 顺序访问 owner/requester，reachable Upvalue
  会向 canonical tracer 发布 owner handle，arena drain 在 generation
  advance/retirement 前完成关闭。
  coroutine create/wrap 现由 exact-id `PendingState` 事务持有未发布 arena
  slot；Thread/closed Upvalue/wrapper Function 同时保留对象临时根，完成
  State↔Thread 双向绑定并直接压入 caller stack 后才提交。提前返回或 panic
  会删除 slot 并推进/退休 generation，canonical tracer 也可在尚无 Thread
  边时从 `TEMPORARY_STATE_ROOTS` 独立到达该状态。
  compiler builder 也已改用 sealed publication allocator：interned string、
  child Proto、top Proto 与 Lua Function 在错误返回或 panic 时均由事务清理，
  成功路径只在 CLI/bytecode explicit root 或 base/package 活动栈接管后释放。
  标准库共享 registration layer 现保护 destination Table、canonical key、
  C/Runtime-native Function 与 environment；catalog library Table 以及
  package/loaded/preload/nested-module/metadata/metatable/error string 也只在
  traced Table 或 stack 接管后释放，foreign edge 在 mutation 前拒绝。
  IO file Userdata、state/metatable、method Function/key、`__index` self-edge
  以及 lines Function→environment→file 图也已在同一事务内构造并直接发布到
  rooted `io` Table 或活动 stack；IO 后续字段/result/error string 通过 typed
  Table/stack publication，生产 `io.rs` 已无直接 `gc.create`。
  VM `NEWTABLE/CLOSURE`、concat/debug/error string、legacy arg Table、open
  Upvalue、Runtime global/registry/error、CLI `arg`/script varargs 以及其余标准库
  result Table/Function/string/Userdata 现也在 durable stack/state/Table 或
  explicit root 接管前保持事务根。原 `call_value -> Vec<Value>` 已由同步
  `call_value_with_results` 替代；callee stack 清空后，collectable results
  在 publication callback 完成前仍由 exact-id temporary roots 保护。
  M1 静态门会排除 `#[cfg(test)]` 后检查目标生产前缀，并拒绝直接
  `gc.create` 或旧 `call_value` API。唯一 `RuntimeStorage -> Heap ->
  {GarbageCollector, StringPool}` owner 图现以 HeapId 防错配；LuaState 不再
  保存 GC/StringPool backpointer，而是在单 state turn 内使用可展开的动态
  service scope。固定字符串、pending-finalizer root seed 与 activation
  service 已由 canonical Runtime tracer 标记，54-path heap contract 防止生产
  路径恢复 standalone 构造或提前公开/接线 STW。
  Runtime full STW 现消费该 tracer；对 gap/foreign edge fail-closed，在
  object sweep 前完成 finalizer prepare/resurrection propagation、weak
  reconciliation，并关闭不可达 state 的 open Upvalue/失效 handle generation，
  再通过 Heap/StringPool sweep 全图和 protected callback delivery。
  但 main state 仍是 external arena slot，debug/protected-helper 跨 state
  open-Upvalue 访问尚未纳入同一调度协议；IO/module service drain、
  allocator live/peak 与 allocation-triggered automatic collection 合同也未
  闭环。production barrier、explicit incremental collection 和生产字符串
  Eq/Hash/canonical/scoped access 已由独立 inventory、静态门和回归本地闭合。
- **Oracle：** `lua_cpp@87c15e6` 的 EngineContext/state ownership、
  close、coroutine lifecycle 和 allocator live-byte 合同。
- **测试与任务：** 1000 轮 state/coroutine create-close、fixed/ordinary
  DropProbe、并发 RuntimeId 唯一性、MAX-generation retirement、free-list
  preflight 与关闭归零已通过。`coroutine-normal-ancestor.lua` 及其独立
  characterization 工具进一步锁定了固定 C++ 允许 `A→B→A` 激活环、而
  stock Lua 拒绝 `Normal` 祖先的差异；Rust process regression 已逐字节
  对齐该 C++ 行为。另有 suspended-coroutine closure 远端读写、
  Upvalue→owner-state root fixed point、集合去重/排序与
  close-before-generation-invalidation 回归；PendingState 故障注入还覆盖
  Thread 分配、slot 插入、双向绑定、压栈提交、exact-id mismatch、panic
  cleanup 与 MAX-generation 单次退休。IO 回归另覆盖 file/iterator 图的
  mark-only 可达性与全回收、method/`__index`/metatable/environment 故障点、
  early return/panic 临时根归零、foreign/stale mutation rejection，以及从
  Runtime main stack 出发的 canonical mark-only tracing。
  VM/app/result 回归还覆盖 result slice mark seed、foreign/panic exact-id 清理、
  callee stack 恢复后的同步发布，以及 CLI `arg` 与 chunk varargs 的进程级
  handoff；目标生产文件直接 `gc.create` 静态命中为 0。
  Heap/VM 回归覆盖同 Heap canonical identity、跨 Heap service 错配拒绝、
  nested/panic service scope、固定字符串 root/close 和 standalone collector
  Drop reclaim；full STW 回归另覆盖两轮全图 sweep、weak v/k/kv、
  protected/reentrant finalizer、queue retention、resurrection/再次死亡、
  typed Userdata Drop、reachable/unreachable state、Upvalue
  close-before-handle-invalid、cross-collector/root-gap/phase gates；heap
  contract 已接入 M1 foundation gate。
  allocator live/peak、service close、Miri/ASan 等仍在
  M1.4、M1.5、M1.7、M1.8、M1.10、M1.11、M1.13。
- **影响：** 已移除 resume/wrap 递归跨 state 借用和 raw open-Upvalue
  owner 风险，并能声称当前 Rust-owned 对象的 deterministic shutdown
  substrate；当前 production publication、唯一 Heap/service owner、full STW、
  weak/finalizer/resurrection 与 public collector 切片已关闭，但剩余
  main-state、allocator/service drain、barrier 与 incremental 风险仍不允许
  声称完整 Lua lifecycle。
- **处置状态：** `open`。temporary state、compiler Proto→Function、
  library/package、IO、VM/app/result publication、字符串 identity/access 与
  唯一 Heap/service owner、全图 collection、weak/finalizer/resurrection、
  public collection 与 Lua `__gc` close drain 子项已关闭；debug/protected-helper
  跨 state、service/allocator close、barrier/incremental 与完整 lifecycle 验收
  全部完成前保持开放。

### NOTE-010: Lua 字符串 intern hash 选择固定 C++ 的前向采样

- **日期：** 2026-07-26
- **范围：** `GcString`/StringPool 的预计算 intern hash；不涉及
  `ByteString` 实现 Rust `Hash` trait 的标准集合语义。
- **设计依据：** [ByteString RFC](byte_string_rfc.md#hashing-policy)。
- **已确认差异：** 固定的 `lua_cpp@87c15e6` 从前向索引采样字符串字节；
  stock Lua 5.1 与迁移前 Rust 实现使用后向采样。64 位 `size_t` 合同下，
  最小向量 `b"ab"` 的 C++ hash 为 `5193`，stock/旧 Rust hash 为 `5161`。
- **目标行为：** 项目优先完整复刻固定 C++ oracle，因此 M1 的专用 Lua intern
  hash 选择前向采样并锁定 `5193` 向量；这不会改变 `ByteString` 对完整逻辑
  bytes 的 `Eq`/`Hash` 合同。
- **Oracle：** `lua_cpp@87c15e6` 的 GCString hash 为项目目标；stock Lua
  5.1.5 作为已记录的次级差异来源。
- **测试与任务：** M1.1–M1.2；`b"ab"` 的 64 位固定向量、长字符串采样
  corpus、intern identity 和双 oracle 差分。
- **影响：** hash bucket 分布与 stock Lua/旧 Rust 不同；Lua 级字符串相等性
  不应改变，但依赖内部 hash 数值或布局的调试/ABI 观察会看到差异。
- **处置状态：** `implemented-local`。前向采样实现、固定 `b"ab"`/长字符串
  向量、canonical intern identity 与生产构造静态门已完成；整体 ByteString
  迁移仍按 NOTE-007 的 dump/load、C API 和双-oracle 完成条件保持开放。

### NOTE-011: 官方 Lua 5.1 的含 NUL chunk name 会在 C 字符串边界截断

- **日期：** 2026-07-26
- **范围：** `load` 的 chunk name、`debug.getinfo().source` 与
  `short_src`；不涉及 Lua 字符串值本身的 NUL 语义。
- **已确认差异：** stock Lua 5.1.5 将传入 loader 的 chunk name 经过
  NUL 结尾的 C 字符串边界，嵌入 NUL 后的字节不会出现在调试 source 中；
  固定的 `lua_cpp@87c15e6` 保留完整 pointer+length 字节串。
- **目标行为：** 本项目以 `lua_cpp` 为项目特有行为 oracle，因此
  `lua_rust` 保留完整含 NUL chunk name；stock lane 的精确输出差异作为
  已批准双-oracle deviation，不得扩展到其他字段、case 或退出状态。
- **证据：**
  `tests/lua/differential/m1-byte-chunk-source.lua`；
  `tests/compatibility/m1-byte-differential-cases.json`。manifest 以
  stdout 的 raw Base64、SHA-256 和唯一字段集合锁定差异，runner 必须拒绝
  未消费或范围变化的 expected difference。
- **影响：** 与 stock Lua 相比，错误/调试信息会保留 NUL 后的 chunk-name
  字节；与 `lua_cpp` 一致。文件内容、普通字符串和退出状态不受此条批准。
- **处置状态：** `approved`。这是目标行为选择，不是允许任意 chunk/source
  输出归一化；若 `lua_cpp` 基线改变，必须重新走 oracle 变更流程。

## Registry

| ID | Area | Oracle | Task | Status |
|---|---|---|---|---|
| NOTE-001 | base / version | `lua_cpp@87c15e6` 优先于 stock | M0.5, M2.8 | `approved` |
| NOTE-002 | GC | stock + `lua_cpp@87c15e6` | M1.7–M1.13 | `open` |
| NOTE-003 | binary chunk | stock + `lua_cpp@87c15e6` | M1.6, M3.1–M3.3 | `mitigated-open` |
| NOTE-004 | stdio | stock + `lua_cpp@87c15e6` | M0.4, M0.5, M2.12 | `remediation-verified-local` |
| NOTE-005 | native module | stock + `lua_cpp@87c15e6` | M3.9 | `open` |
| NOTE-006 | OS / locale / time | per-platform stock + C++ | M2.13, M5.1 | `open` |
| NOTE-007 | byte strings | stock + `lua_cpp@87c15e6` | M1.1–M1.3, M2.10 | `mitigated-open` |
| NOTE-008 | C API / ABI | stock + `lua_cpp@87c15e6` | M3.4–M3.8 | `open` |
| NOTE-009 | ownership / lifecycle | `lua_cpp@87c15e6` | M1.4–M1.8, M1.13 | `open` |
| NOTE-010 | string intern hash | `lua_cpp@87c15e6` 优先于 stock | M1.1–M1.2 | `implemented-local` |
| NOTE-011 | NUL chunk-name boundary | `lua_cpp@87c15e6` 优先于 stock | M1.3, M2.1, M2.7 | `approved` |

## 维护规则

1. expected failure 必须引用一个 `NOTE-*` 或 `plan.md` 中的任务 ID。
2. `approved` 只表示目标行为已决定，不表示实现或测试已经通过。
3. 将条目标为 `resolved` 时，必须补充合入提交、oracle 命令和 artifact 路径。
4. 不删除历史条目；行为恢复对齐后保留 ID 并修改状态。
