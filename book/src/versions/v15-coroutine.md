# v0.15 · Coroutine

<div class="lesson-meta"><span>核心增量：可暂停、可恢复的独立执行状态</span><span>workspace</span><span>checkpoint v0.15</span></div>

v0.14 的 VM 只有一条正在前进的执行链：函数要么返回，要么报错。本版增加一个核心能力：**函数可以在任意普通 Lua 调用深度暂停，把控制权和值交给恢复者，之后从原指令位置继续执行**。实现保存真实 VM 帧，不通过重跑源码伪造恢复，因此 yield 之前的赋值、IO 占位和 GC 副作用只发生一次。

## 线程值与 owner turn

`Value::Coroutine` 是 Lua 的 `thread` 值，身份来自 Runtime heap；`CoroutineState` 则保存在 VM 调度器中。二者分工如下：

- Heap 对象参加可达性分析，并持有暂停状态里所有 Lua 值的 tracing roots；
- VM 状态保存 `Suspended / Running / Normal / Dead`、入口函数和暂停帧；
- `current_coroutine` 是当前 owner turn；嵌套 `resume` 时父线程由 `Running` 临时变为 `Normal`；
- 主线程没有伪造的 thread handle，因此 `coroutine.running()` 在主线程返回 nil。

```rust
{{#include ../../code/v15-coroutine/crates/lua_vm/src/lib.rs:coroutine_state}}
```

这种拆分避免把 Rust 调用栈当作长期存储。暂停后 Rust 函数已经返回，真正需要延续的内容必须显式拥有。

## Yield 如何穿过调用栈

`coroutine.yield(...)` 不返回普通结果，而是产生只在 VM 内部识别的 `VmErrorKind::Yield`。每个遇到该信号的 `CALL` 保存：

- 当前 Lua function 与稳定 `frame_id`；
- register slots、vararg 和下一条 `pc`；
- 恢复值应该写入的 base 与固定/开放结果数量；
- tail call 是否应直接把恢复值作为当前帧结果。

内层先保存，外层后保存，所以列表自然形成 `[最内层, ..., 最外层]`。暂停路径从 active roots 移除帧，却不关闭 open upvalue；正常返回和普通错误路径才执行 close。

恢复时，第一组参数就是最内层 `yield` 表达式的返回值。若该帧正常返回，它的结果继续写入下一个外层帧的调用点，直到回到 coroutine 入口：

```text
resume(args)
    │
    ▼
[inner frame] ──return values──> [caller frame] ──...──> coroutine return
    │ yield
    └──保存新 inner frames + 尚未恢复的 outer frames
```

```rust
{{#include ../../code/v15-coroutine/crates/lua_vm/src/lib.rs:coroutine_resume}}
```

这个过程也覆盖多返回和 tail-yield：开放结果原样传播，tail 位置恢复后直接成为函数返回值。

## API 合同

本版注册完整的 coroutine 表：

| API | 成功结果 | 失败行为 |
|---|---|---|
| `create(f)` | suspended thread | 非 function 为类型错误 |
| `resume(co, ...)` | `true, yielded-or-returned...` | `false, error`，线程变 dead |
| `yield(...)` | 下次 resume 的参数 | 主线程或禁止边界为错误 |
| `status(co)` | running/normal/suspended/dead | 非 thread 为类型错误 |
| `running()` | 当前 thread；主线程为 nil | 无 |
| `wrap(f)` | 去掉成功布尔值的 function | resume 错误重新抛出 |

`create`/`wrap` 同时接受 Lua function 和已有 builtin function。恢复 running、normal 或 dead thread 不执行任何字节码，只返回 `false, message`。

Lua 5.1 不允许 yield 跨越不可延续的 C/元方法边界。本版用 `yield_forbidden_depth` 明确保护算术、索引、赋值、`__call` 和 finalizer 回调；从这些回调 yield 会使目标 coroutine 以分类错误结束，而不会留下半暂停的帧。

## Coroutine 与 GC

暂停寄存器已经不在 `active_frames` 中，若不添加新边，增量 GC 会把仍需恢复的 table、closure 或 upvalue 当成垃圾。本版在每次 suspension 后把以下值同步到 Heap thread object：

- 尚未首次执行的入口 function；
- 每个暂停帧的 function、register values 和 varargs；
- closure 所引用的 open/closed upvalue 由 Heap 图继续追踪。

因此 `coroutine handle → suspended roots → closure/table/upvalue` 构成普通强可达图。若 handle 本身不可达，collector 不访问其内部 roots；Sweep 后 VM 清理对应调度状态与 open-upvalue 索引。测试同时证明“可达暂停线程跨完整 GC 存活”和“不可达暂停线程连同自引用 table 被回收”。

只把 active frame 的 open upvalue 作为额外 root 很重要：若把全局 open-upvalue map 全部播种为 roots，不可达的 suspended coroutine 会被这张宿主索引表永久泄漏。

## Rust 知识映射与设计选择

- `CoroutineStatus` enum 把合法状态迁移变成可检查协议；
- `SuspendedFrame` 把原本隐含在 Rust 调用栈中的 continuation 数据显式化；
- `std::mem::take` 让嵌套 resume 临时取得子线程状态，同时保留父线程的 yield 通道；
- typed `CoroutineRef` 保证 thread/table/function 句柄不会混用；
- Heap object 持有 traced `Vec<Value>`，VM map 持有执行细节，避免 GC 依赖 Rust 容器的所有权偶然性；
- 内部 Yield 信号复用结构化错误传播，但在 `resume` 边界必定被消费，不会泄漏到宿主 API。

替代方案一是把每条指令实现为可序列化状态机，但会让所有 opcode 承担 continuation 分支；替代方案二是为每个 coroutine 创建 OS thread 或 async task，但那会把 Lua 的协作式调度错误映射成宿主并发。本版保存 VM frame，语义最直接，也为后续 debug stack 留下稳定数据。

## 从 v0.14 迁移了什么

- 新增：`thread` / coroutine wrapper 值、table key 身份和 heap tracing；
- 新增：六个 Lua 5.1 coroutine API 与状态迁移；
- 重构：调用帧 cleanup 区分 yield 与 return/error；
- 新增：普通 CALL、开放结果和 tail call 的 continuation 描述；
- 新增：嵌套 resume 的 owner turn 与 `normal` 状态；
- 重构：GC roots 只直接包含 active upvalue，暂停 roots 改由 thread 对象拥有；
- 新增：不可达 coroutine 状态清理、跨 GC/跨宿主执行恢复测试；
- 保留：v0.14 incremental GC、weak table、finalizer 和诊断 helper；
- 没有提前加入：标准库 crate、模块系统、REPL 或 bytecode 工具。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v15-coroutine/Cargo.toml -- `
  book/code/v15-coroutine/examples/demo.lua
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --workspace
```

示例经历两次 yield，中途执行完整 GC，最后从保存的 frame 返回：

```text
type=number value=1515
```

本快照共有 99 个测试：74 个端到端测试、7 个编译器测试、15 个 core 测试和 3 个 VM 测试。新增覆盖生成器、多值交换、嵌套 resume、四种 status、wrap、builtin 入口、自恢复拒绝、闭包/open upvalue、深调用链、tail-yield、暂停 GC roots、不可达状态清理、跨宿主执行、错误传播和禁止 yield 边界。

## 本版本暂不支持

- 没有 `pcall` / `xpcall`，wrap 错误只能交给宿主或未来的受保护调用捕获；
- traceback 尚未保存每次 suspension 时完整的 debug call-info，恢复后的错误定位仍需 v0.17 debug 库收口；
- 没有 debug hook 对 yield/resume 的事件协议；
- coroutine 没有显式 close API；Lua 5.1 本身也没有 `coroutine.close`；
- finalizer/userdata/GC 入口仍是教学 helper，不是最终标准库命名；
- 没有 base、math、string、table 等核心标准库适配器。

## 章末练习

1. 删除 Heap thread 的 register roots，在暂停帧中保存自引用 table，复现 GC 后的 stale handle。
2. 为 `SuspendedFrame` 增加 debug call-info，让恢复后错误包含暂停前的完整 caller chain。
3. 写三层 A → resume(B) → resume(C)，在每一层验证 running/normal 状态和返回值方向。
4. 让 `__call` 元方法尝试 yield，再移除 boundary guard，解释为什么结果偏离 Lua 5.1。
5. 用 `VecDeque` 替换当前教学用的帧头部移除，测量一千层恢复链的差异。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 coroutine owner turn、独立 stack/call-info 和 yield boundary 是本版显式状态的性能化目标；
- [Closure 与 Upvalue](v11-closure-upvalue.md) 可回看为什么 suspension 不能提前 close open upvalue；
- [增量 GC](v14-incremental-gc.md) 可对照 thread roots 如何进入同一三色标记协议；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可定位 base、开放结果与 tail call 的原始执行合同；
- 下一 checkpoint 是 v0.16：把宿主函数适配器独立成 `lua_stdlib`，系统实现 base、math、string、table 和 coroutine 库。
