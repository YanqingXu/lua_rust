---
status: partial
phase: 1
phase_name: Runtime Core
last_updated: 2026-07-29
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 1 Report: Runtime Core

## 结论

**状态：partial。** ByteString、checked `GcRef` provenance、managed Proto、
StateArena/StateHandle、checked open-Upvalue owner、Runtime coroutine
trampoline、`TEMPORARY_STATE_ROOTS/PendingState`、canonical mark-only
tracer、compiler Proto→Function publication 与确定性 shutdown substrate 已有本地
实现和回归；library/package 与 IO construction graph publication 也已迁移，但
唯一 Heap owner、VM/app/results 生产 publication、真实 GC 回收、write
barrier 和完整 Lua-visible shutdown 尚未闭环。因此，本阶段不能标记
completed。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | 无。尚没有满足双 oracle 与阶段完成门槛的整项能力。 |
| `partial` | Value/GC object、table/metatable、function/proto/upvalue、userdata/thread、Runtime/StateArena、mark-only root/shutdown 与 weak/finalize/sweep 组件。 |
| `not-started` | 真实 Runtime-only full/incremental collection、完整 barrier/finalizer/resurrection、allocator soak 与 Lua C API owner 合同。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Runtime value model | `crates/lua_core/src/value.rs`、`types.rs` | `crates/lua_core/tests/value_tests.rs` 及模块内单元测试 | partial：存在项目内测试，尚无完整 C API/type differential |
| GC object/header/ref | `crates/lua_core/src/gc/header.rs`、`gc_ref.rs`、`gc_object.rs` | ObjectId/live-table provenance、foreign/stale/type rejection 与 GC 模块测试 | partial：唯一 Heap owner、全部 scoped borrow/publication 尚未完成 |
| Table/metatable | `table.rs`、`metatable.rs` | table/value 项目内测试和 Lua fixtures | partial：write barrier、GC 周期和 oracle 边角未验证 |
| Function/Proto/upvalue | `function.rs`、`proto.rs`、`upvalue.rs`、compiler/library/IO publication、Runtime root/transfer 路径 | managed Proto、compiler string/child-Proto/top-Function、library/package Function、IO lines Function/environment/file graph 事务发布、checked owner、跨 state GET/SET、root fixed-point 与 close-order 回归 | partial：VM/app/results publication、debug/protected-helper 跨 state、barrier 与 live sweep 未闭环 |
| Userdata/thread/state | `userdata.rs`、`thread.rs`、`io.rs`、`lua_vm::runtime` | IO file Userdata/metatable/method graph、StateHandle identity/retirement、PendingState exact-id rollback/root seed、coroutine create/wrap transactional publication、trampoline、typed DropProbe 与 1000 轮 shutdown | partial：finalizer/service close、main owner 与其余生产 publication 未闭环 |
| Mark/weak/finalize/sweep 组件 | `crates/lua_core/src/gc/mark.rs`、`weak.rs`、`finalize.rs`、`sweep.rs` | 组件级单元测试 | partial：stdlib GC 控制路径未执行真实 sweep |

这里的“验证证据”只证明 Rust 内部合同被测试，不等价于 stock Lua 5.1 或
`lua_cpp` 的可观察行为已经一致。

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| 生产字符串尚未全部强制 canonical interning/scoped Eq/Hash | ByteString 已保留任意 bytes，但仍有直接 `GcString` publication 与无 collector borrow 的内容 Eq/Hash | [NOTE-007](deviation_log.md#note-007-lua-字符串尚未采用任意字节表示)、[NOTE-010](deviation_log.md#note-010-lua-字符串-intern-hash-选择固定-c-的前向采样)，M1.2–M1.3 |
| `collectgarbage` 没有运行完整 sweep，计数和 step 为模拟值 | `base.rs` 的 `poll_gcinfo_kb`/`step_gcinfo_cycle`；collector Drop 不析构对象 | [NOTE-002](deviation_log.md#note-002-gc-可观察行为尚未形成真实回收闭环)，M1.7–M1.13 |
| 写屏障函数存在但 mutation site 仍有 TODO | table/function/upvalue/userdata/thread 的 setter 注释 | NOTE-002，M1.11 |
| Runtime/Heap 与 publication 所有权未闭环 | `LuaState` 仍保存 transitional service backpointer，main state 为 external arena slot；temporary state/coroutine、compiler Proto→Function、library/package 与 IO graph publication 已迁移，但 VM/app/results 等对象图仍未迁移 | [NOTE-009](deviation_log.md#note-009-runtime-与-coroutine-所有权未闭环)，M1.4–M1.8 |
| Binary chunk 尚未实现 | M1.6 已删除 dump registry，并让 `string.dump` 明确报 unsupported；真实格式仍缺失 | [NOTE-003](deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk)，M3.1–M3.3 |

## Oracle 与验收

- Lua 语义 oracle：官方 Lua 5.1.5。
- 项目扩展与生命周期 oracle：`lua_cpp@87c15e6`。
- M0 的单一 weak-value differential 本地通过；它只证明该最小 observable，
  不能替代本报告列出的真实 sweep、计账和 lifecycle 门槛。
- 当前 766 项 workspace tests、24/24 inventory 和 shutdown/owner 回归不能
  替代真实回收、allocator live bytes、weak/finalizer/resurrection 和
  sanitizer/Miri 证据。
- 完成条件以 [`plan.md` M1](../../plan.md#6-m1字节字符串所有权与真实-gc)
  为准；所有门槛通过前保持 `partial`。
