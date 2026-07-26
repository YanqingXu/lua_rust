---
status: partial
phase: 1
phase_name: Runtime Core
last_updated: 2026-07-26
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 1 Report: Runtime Core

## 结论

**状态：partial。** Runtime Core 的主要数据结构和 GC 组件已经存在，也有项目内
单元测试；但字节字符串、运行时唯一所有者、真实 GC 回收、write barrier 和确定性
shutdown 尚未闭环。因此，本阶段不能标记 completed。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | 无。尚没有满足双 oracle 与阶段完成门槛的整项能力。 |
| `partial` | Value/GC object、table/metatable、function/proto/upvalue、userdata/thread、mark/weak/finalize/sweep 组件。 |
| `not-started` | 统一 ByteString、Runtime/EngineContext owner、可审计的 coroutine arena 和完整 lifecycle/allocator soak 证据。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Runtime value model | `crates/lua_core/src/value.rs`、`types.rs` | `crates/lua_core/tests/value_tests.rs` 及模块内单元测试 | partial：存在项目内测试，尚无完整 C API/type differential |
| GC object/header/ref | `crates/lua_core/src/gc/header.rs`、`gc_ref.rs`、`gc_object.rs` | GC 模块内单元测试 | partial：unsafe/lifetime 和跨 collector 合同未完成 |
| Table/metatable | `table.rs`、`metatable.rs` | table/value 项目内测试和 Lua fixtures | partial：write barrier、GC 周期和 oracle 边角未验证 |
| Function/Proto/upvalue | `function.rs`、`proto.rs`、`upvalue.rs` | core/compiler/VM 项目内测试 | partial：生命周期与 barrier 未闭环 |
| Userdata/thread | `userdata.rs`、`thread.rs` | core 与 coroutine 项目内测试 | partial：finalizer、coroutine state 释放和 shutdown 未证明 |
| Mark/weak/finalize/sweep 组件 | `crates/lua_core/src/gc/mark.rs`、`weak.rs`、`finalize.rs`、`sweep.rs` | 组件级单元测试 | partial：stdlib GC 控制路径未执行真实 sweep |

这里的“验证证据”只证明 Rust 内部合同被测试，不等价于 stock Lua 5.1 或
`lua_cpp` 的可观察行为已经一致。

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| Lua string 由 `String`/`&str` 表示，不能保留任意 bytes | `GcString::data()` 返回 `&str`，多个模块使用 char/UTF-8/Latin-1 转换 | [NOTE-007](deviation_log.md#note-007-lua-字符串尚未采用任意字节表示)，M1.1–M1.3 |
| `collectgarbage` 没有运行完整 sweep，计数和 step 为模拟值 | `base.rs` 的 `poll_gcinfo_kb`/`step_gcinfo_cycle`；collector Drop 不析构对象 | [NOTE-002](deviation_log.md#note-002-gc-可观察行为尚未形成真实回收闭环)，M1.7–M1.13 |
| 写屏障函数存在但 mutation site 仍有 TODO | table/function/upvalue/userdata/thread 的 setter 注释 | NOTE-002，M1.11 |
| Runtime 服务由裸指针拼装 | `LuaState` service pointers、coroutine `Box::into_raw` | [NOTE-009](deviation_log.md#note-009-runtime-与-coroutine-所有权未闭环)，M1.4–M1.8 |
| Binary chunk 尚未实现 | M1.6 已删除 dump registry，并让 `string.dump` 明确报 unsupported；真实格式仍缺失 | [NOTE-003](deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk)，M3.1–M3.3 |

## Oracle 与验收

- Lua 语义 oracle：官方 Lua 5.1.5。
- 项目扩展与生命周期 oracle：`lua_cpp@87c15e6`。
- M0 的单一 weak-value differential 本地通过；它只证明该最小 observable，
  不能替代本报告列出的真实 sweep、计账和 lifecycle 门槛。
- 当前已有的 workspace 测试基线不能替代 root inventory、真实回收、allocator
  live bytes、1000 轮 state/coroutine create-close、weak/finalizer/resurrection
  和 sanitizer/Miri 证据。
- 完成条件以 [`plan.md` M1](../../plan.md#6-m1字节字符串所有权与真实-gc)
  为准；所有门槛通过前保持 `partial`。
