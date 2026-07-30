---
status: partial
phase: 1
phase_name: Runtime Core
last_updated: 2026-07-30
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 1 Report: Runtime Core

## 结论

**状态：partial。** ByteString、checked `GcRef` provenance、managed Proto、
StateArena/StateHandle、checked open-Upvalue owner、Runtime coroutine
trampoline、debug/protected Runtime-native upvalue scheduling、
`TEMPORARY_STATE_ROOTS/PendingState`、canonical mark-only
tracer、compiler Proto→Function publication 与确定性 shutdown substrate 已有本地
实现和回归；library/package、IO、VM/app 与同步 result construction publication、
production string canonical identity/scoped byte access，以及唯一 Heap/service
owner 也已迁移。Runtime safe-point full STW 已消费 canonical tracer，执行
finalizer prepare/resurrection propagation、weak reconciliation、不可达 state
prepass、真实 sweep/accounting 和 protected callback delivery；Lua-visible
`collectgarbage("collect"/"step")`、五阶段 explicit incremental cycle、
8-family checked mutation barrier、实际 `gcinfo/count` 与 close drain 已接线。
但 allocation-triggered automatic GC、allocator accounting 和完整 shutdown
尚未闭环。
因此，本阶段不能标记 completed。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | 无。尚没有满足双 oracle 与阶段完成门槛的整项能力。 |
| `partial` | Value/GC object、table/metatable、function/proto/upvalue、userdata/thread、Runtime/StateArena、canonical roots、全图 STW、shutdown 与 weak/finalize/sweep 组件。 |
| `not-started` | allocation-triggered automatic collection、allocator soak 与 Lua C API owner 合同。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Runtime value model | `crates/lua_core/src/value.rs`、`types.rs` | `crates/lua_core/tests/value_tests.rs` 及模块内单元测试 | partial：存在项目内测试，尚无完整 C API/type differential |
| GC object/header/ref | `crates/lua_core/src/gc/header.rs`、`gc_ref.rs`、`gc_object.rs`、`heap.rs` | HeapId、ObjectId/live-table provenance、foreign/stale/type rejection、standalone Drop reclaim、result-slice exact-id root 与 GC 模块测试 | partial：唯一 Heap owner 已完成；通用非字符串 object-scoped execution context 尚未完成 |
| String identity/access | `value.rs`、`string_pool.rs`、`collector.rs`、`LuaState` 与 compiler/VM/stdlib/app 构造边界 | 17-path [`string_access_inventory.json`](../../tests/compatibility/string_access_inventory.json)、静态合同门、duplicate/foreign/stale/NUL/high-byte/address-reuse 回归 | completed-local：生产构造强制 canonical pool identity，safe Eq/Hash 不解引用，内容读取经 collector/state scope；dump/load 与未来 C API 仍由 M1.3/M3 跟踪 |
| Table/metatable | `table.rs`、`metatable.rs` | table/value 项目内测试、Lua fixtures、8-family mutation inventory 和 directed black-to-white 回归 | partial：checked write barrier 与 explicit incremental cycle 已落地；动态容量计账和 oracle 边角未验证 |
| Function/Proto/upvalue | `function.rs`、`proto.rs`、`upvalue.rs`、compiler/library/IO/VM publication、Runtime root/transfer 路径 | managed Proto、compiler string/child-Proto/top-Function、library/package Function、IO iterator graph、VM closure/open-Upvalue 与 synchronous result 事务发布、跨 state GET/SET、debug direct/`pcall` Runtime-native owner turn、foreign/stale error response、root fixed-point、STW/incremental prepass 与 close-order 回归 | partial：sealed debug get/setup 的跨 state 与 protected delivery 已闭合；任意 Lua protected-helper suspension、通用非字符串 scoped access 与深链 fault 矩阵仍未闭环 |
| Userdata/thread/state | `userdata.rs`、`thread.rs`、`io.rs`、`lua_vm::runtime` | IO/proxy Userdata graph、StateHandle identity/retirement、PendingState rollback/root seed、coroutine create/wrap、CLI argument handoff、trampoline、scoped services、unreachable-state prepass、protected/close finalizer drain、typed DropProbe 与 1000 轮 shutdown | partial：唯一 Heap/service owner、finalizer drain 与 internal state reclaim 已完成；service close 与 main-state arena slot 统一仍未闭环 |
| Mark/weak/finalize/sweep 组件 | `gc/mark.rs`、`gc/incremental.rs`、`weak.rs`、`finalize.rs`、`sweep.rs`、`runtime/full_collection.rs`、`runtime/incremental_collection.rs` | 两轮全图 sweep、五阶段/有界 work unit、weak v/k/kv、pending/finalized userdata、protected error、queue retention、nested collect、resurrection/再次死亡、reachable/unreachable object/state、Upvalue close-before-handle-invalid、typed Drop、cross-collector/root-gap/phase gates | completed-local：full STW 与 stdlib `collectgarbage("collect"/"step")` 已执行真实全图回收；automatic allocation checkpoint 仍由后续任务跟踪 |

这里的“验证证据”只证明 Rust 内部合同被测试，不等价于 stock Lua 5.1 或
`lua_cpp` 的可观察行为已经一致。

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| allocator live/peak 尚未闭环 | `gcinfo/count` 当前读取 collector estimated/accounted bytes；尚不能宣称 allocator contract | [NOTE-002](deviation_log.md#note-002-gc-增量与-allocator-可观察行为尚未闭环)，M1.13 |
| allocation-triggered automatic collection 尚未闭环 | Lua-visible full/explicit incremental collection 已消费 canonical tracer、拒绝所有 gap/foreign edge，并完成 mutation barrier、weak/finalizer/resurrection；automatic checkpoint 仍 fail-closed | [NOTE-009](deviation_log.md#note-009-runtime-与-coroutine-所有权未闭环)，M1.13 |
| Binary chunk 尚未实现 | M1.6 已删除 dump registry，并让 `string.dump` 明确报 unsupported；真实格式仍缺失 | [NOTE-003](deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk)，M3.1–M3.3 |

## Oracle 与验收

- Lua 语义 oracle：官方 Lua 5.1.5。
- 项目扩展与生命周期 oracle：`lua_cpp@87c15e6`。
- 含 weak-value 的 4-case differential 在官方 Lua 5.1.5 与固定 C++ 两条
  lane 本地通过；它不能替代本报告列出的计账和 lifecycle 门槛。
- 当前 Debug/Release 各 829 项 workspace tests、24/24 root inventory、
  8/8 mutation inventory、10/10 allocator、54-path heap contract、17-path
  string contract 和 shutdown/owner 回归不能替代通用 non-string scoped
  access、深链 fault matrix 与跨平台 sanitizer 证据。
- 本地 Debug/Release、all-targets Clippy、warning-free rustdoc 与固定
  C++/official 两条 raw-byte differential lane 均通过；M1 smoke 因本机未安装
  `cargo-audit` 而显式跳过 audit，所以只报告 `checksPassed=true`，
  `foundationPassed=false`。
- 完成条件以 [`plan.md` M1](../../plan.md#6-m1字节字符串所有权与真实-gc)
  为准；所有门槛通过前保持 `partial`。
