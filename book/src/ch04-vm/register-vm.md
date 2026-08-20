# 07 · 寄存器虚拟机

<div class="lesson-meta"><span>预计 30 分钟</span><span>Rust：借用边界 / dispatch / Result</span><span>虚拟机</span></div>

## VM 的最小模型

把一台寄存器虚拟机想成三样东西：

1. 一段指令和一个程序计数器 `pc`；
2. 一组保存 `Value` 的寄存器；
3. 一个不断取指、解码、执行的循环。

Lua 函数调用后，当前栈的一段区域被当作寄存器窗口。`R(0)` 不是 CPU 物理寄存器，而是 `base + 0` 对应的 `Value` 槽位。

## 栈是连续 Value 容器

项目中的基础栈结构非常朴素：

```rust
{{#include ../../../crates/lua_vm/src/state/stack.rs:lua_vm_stack}}
```

`push` 在已有缓冲区中复用槽位，必要时扩展 `Vec`；`pop` 用 `Value::Nil` 替换离开逻辑栈的值，避免旧 GC 句柄继续被容器持有。`top` 是逻辑长度，不必等于底层 `Vec` 长度。

调用帧再为这段栈提供 `base`、期望返回值数量和保存的 `pc`。因此同一个连续容器可以承载多层 Lua 调用。

## 主循环：取指、解码、执行

下面是执行循环的开头与四条装载类指令：

```rust
{{#include ../../../crates/lua_vm/src/execute.rs:lua_vm_dispatch}}
```

每轮循环执行相同骨架：

```text
验证当前 Proto → 读取 code[pc] → 解出 opcode/参数 → 操作栈 → pc 前进
```

Rust 的 `match op` 与 opcode enum 一一对应。对于 `LOADK`，Bx 选择常量，A 选择目标寄存器；对于 `LOADBOOL`，C 还决定是否跳过下一条指令。

## 为什么循环里会出现 unsafe

`GcRef<Proto>` 是受运行时管理的句柄。VM 在每次迭代前通过 collector 校验地址、对象身份和类型，然后才把已验证指针转成引用。`unsafe` 没有被假装消失，而是被压缩到一个带明确前置条件的边界。

安全性来自两部分配合：

- Rust 类型与借用规则保护普通控制流；
- collector 的运行时不变量证明该指针在本轮不会失效。

当静态系统无法独自证明动态 GC 图时，好的做法不是到处绕过检查，而是集中、记录并测试必要的 `unsafe`。

## 一次手工执行

假设常量表 `K[0]=40`、`K[1]=2`，指令为：

```text
0  LOADK R(0) K(0)
1  LOADK R(1) K(1)
2  ADD   R(0) R(0) R(1)
3  RETURN R(0) 1
```

状态变化：

| pc | 执行后 R(0) | 执行后 R(1) | 结果 |
|---:|---:|---:|---|
| 0 | 40 | nil | 继续 |
| 1 | 40 | 2 | 继续 |
| 2 | 42 | 2 | 继续 |
| 3 | 42 | 2 | 返回 42 |

真实 `ADD` 还要处理类型错误和 `__add` 元方法，`RETURN` 还要处理多返回值与调用帧恢复。简单模型仍然是理解复杂分支的锚点。

## 错误为什么继续用 Result

执行用户程序失败是普通结果。`execute_proto` 返回 `Result<ExecResult, RuntimeError>`：成功时可能正常返回或 yield，失败时携带 Lua 运行时错误。`?` 让深层表访问、元方法或调用逻辑把错误传回入口，同时保留清理当前帧的机会。

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>用表格手工执行 <code>LOADK, LOADK, MUL, RETURN</code>。</li>
    <li>在 <code>execute.rs</code> 中找到 <code>ADD</code>，区分数字快路径与元方法路径。</li>
    <li>思考 <code>pc</code> 应在 match 前还是后递增；跳转指令会如何影响选择？</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>寄存器是栈窗口中的 Value 槽；dispatch 循环按 opcode 改变状态；复杂 Lua 行为最终都必须落到可观察的寄存器、调用帧、pc 或运行时对象变化。</p>
</div>

继续探索：[`execute.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_vm/src/execute.rs) · [`state/`](https://github.com/YanqingXu/lua_rust/tree/main/crates/lua_vm/src/state)
