---
status: partial
phase: 3
phase_name: Virtual Machine
last_updated: 2026-07-26
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 3 Report: Virtual Machine

## 结论

**状态：partial。** VM dispatch 包含 Lua 5.1 opcode 枚举对应的执行分支，函数调用、
多返回、闭包/upvalue、循环、metamethod 和 coroutine 的主要项目内路径可运行。
但尚无 38 opcode 全量 bytecode/trace parity，GC 与 coroutine 所有权也会影响
可观察语义，因此不能标记 completed。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | 无。尚没有 VM 子域通过完整 bytecode/trace 双 oracle 门槛。 |
| `partial` | dispatch、stack/call、closure/upvalue、metamethod、coroutine/debug 主要路径。 |
| `not-started` | 38-opcode 全量结构化 parity、完整 host ABI 合同与 owner-safe coroutine lifecycle。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Opcode dispatch | `crates/lua_vm/src/execute.rs` 对 opcode enum 分派 | VM integration tests、按语言功能组织的 Lua fixtures | partial：尚无逐 opcode trace 证据 |
| Stack/call frames | `crates/lua_vm/src/state/` | VM integration tests | partial：stack overflow、深度和重入合同未与 oracle 对齐 |
| Calls/returns/vararg/tailcall | CALL/TAILCALL/RETURN/VARARG 路径 | functions/regressions fixtures | partial：host boundary、error/yield 和 debug event 仍缺 differential |
| Closure/upvalue | CLOSURE、open/close upvalue 路径 | closure/upvalue regressions | partial：coroutine 穿越与生命周期证明不完整 |
| Metamethod dispatch | arithmetic/index/newindex/call/compare/concat/len 等路径 | table/metatable tests 与 fixtures | partial：链循环限制、错误文本和 primitive metatable 未全量对齐 |
| Coroutine/debug | resume/yield/status 与部分 debug API 路径 | stdlib/VM 项目内测试 | partial：owner/lifetime、hook 与错误状态矩阵未闭环 |

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| 缺少可靠的 bytecode/VM trace parity artifact | 当前 M0 正在恢复 runner，尚无全量 38-opcode 通过报告 | M0.6、M2.2 |
| 调用/返回和错误边界未全面差分 | 现有 tests 主要验证 Rust 自身期望 | M2.4 |
| Upvalue 与 coroutine 跨 yield 生命周期未证明 | coroutine state 由裸 `LuaState` pointer 持有 | [NOTE-009](deviation_log.md#note-009-runtime-与-coroutine-所有权未闭环)，M1.5、M2.5、M2.7 |
| Metamethod 与 debug 细节未完成 | 缺少循环限制、source line、hook event、tail event 的 oracle matrix | M2.6、M2.7 |
| VM 内运行 GC 不执行真实回收 | `collectgarbage` 路径未 sweep，barrier 未接入全部 mutation sites | [NOTE-002](deviation_log.md#note-002-gc-可观察行为尚未形成真实回收闭环)，M1.9–M1.12 |
| Host ABI 不存在 | 内部 C-function pointer 不是 Lua C API | [NOTE-008](deviation_log.md#note-008-lua-51-c-apiabi-尚不存在)，M3.4–M3.8 |

## Oracle 与验收

- VM 值/错误/trace 与官方 Lua 5.1.5 和 `lua_cpp@87c15e6` 双向比较。
- 当前 fixture/VM tests 是回归资产；在 manifest、进程级 runner 和 trace
  comparison 完成前，不把“脚本可运行”提升为“语义一致”。
- 完成条件对应 M2.2、M2.4–M2.7 及 M2 总门槛；目前保持 `partial`。
