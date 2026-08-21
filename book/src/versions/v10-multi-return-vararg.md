# v0.10 · 多返回、vararg 与迭代器

<div class="lesson-meta"><span>核心增量：可变长度结果</span><span>workspace</span><span>checkpoint v0.10</span></div>

v0.9 把函数调用压缩成了“若干参数进去、一个值出来”。这便于先看清调用帧，却不是 Lua 的真实协议。本版只推进一个核心概念：**值列表的宽度可以到运行时才确定**。多返回、vararg、尾调用和 generic for 都建立在这个概念之上。

## 本版完成后能做什么

```lua
function values(...)
    return ...
end

local a, b, c = values(10, 20, 30)
for index, value in ipairs({a, b, c}) do
    -- index/value 来自迭代器的多返回结果
end
```

新增能力包括：

- 函数可以返回零个、一个或多个值；
- `...` 只能出现在 vararg 函数中，可以折叠、展开或继续转发；
- 赋值、实参、`return` 和 table 构造器遵循同一套“只有末项展开”规则；
- `return f(...)` 编译为 `TAILCALL`，VM 用 trampoline 替换当前 Lua 帧；
- generic for 消费 `(generator, state, control)` 三元组；
- 教学内建 `next`、`pairs` 和 `ipairs` 提供第一组迭代器。

## 先建立值列表规则

表达式本身没有永久的“多值类型”。它是否展开，由所在的列表上下文决定：

| 代码 | 结果 |
|---|---|
| `a, b, c = f()` | `f` 最多提供三个值，不足补 `nil` |
| `a, b = 1, f()` | `f` 位于末项，可以继续展开 |
| `a, b = f(), 1` | 非末项的 `f()` 折叠成一个值 |
| `a, b = (f())` | 括号强制折叠成一个值 |
| `g(1, f())` | 末尾调用结果全部成为 `g` 的参数 |
| `{1, f()}` | 末尾调用结果全部进入数组部分 |

编译器因此不把“表达式求值”简单建模为 `Expr -> Register`。在需要列表的地方，它还要传入结果宽度：固定数量用 `Some(n)`，开放数量用 `None`。

```rust
{{#include ../../code/v10-multi-return-vararg/crates/lua_compiler/src/lib.rs:result_protocol}}
```

## CALL、RETURN、VARARG 的宽度

本教学 VM 使用以下约定：

- `CALL A B C`：函数位于 `R[A]`；`B-1` 是固定参数数，`B=0` 表示参数一直延伸到 `top`；`C-1` 是固定结果数，`C=0` 表示保留全部结果。
- `RETURN A B`：返回 `B-1` 个固定值；`B=0` 返回 `R[A..top]`。
- `VARARG A B`：复制 `B-1` 个 vararg 并补 `nil`；`B=0` 复制全部并更新 `top`。

这里的 `top` 不是“永远等于寄存器数组长度”。它是当前开放值窗口的右边界。末尾调用或 `VARARG` 写入开放结果后，下一个 `CALL`、`RETURN` 或 `SETLISTOPEN` 才会消费它。

```rust
{{#include ../../code/v10-multi-return-vararg/crates/lua_vm/src/lib.rs:variable_results}}
```

## 为什么尾调用需要 trampoline

如果只是把 `TAILCALL` 实现成 Rust 递归，Lua 栈帧虽然没有增长，宿主线程栈仍会增长。本版让一个执行帧返回 `TailCall(function, arguments)`，外层循环直接换入目标 `Proto` 和参数：

```text
当前 Proto → TAILCALL → FrameOutcome::TailCall
                         ↓
                  替换 Proto/arguments
                         ↓
                    继续同一个循环
```

因此 5000 次尾递归不会触发 Lua 调用深度限制，也不会堆积 Rust 调用栈。普通 `CALL` 仍会增加回溯帧。全局指令预算跨普通调用和尾调用共享，死循环最终仍会得到结构化错误。

## Generic for 的三元组

Lua 的

```lua
for key, value in pairs(table_value) do
    -- body
end
```

先把表达式列表调整成三个值：生成器函数、状态、控制变量。每轮执行等价于：

```lua
key, value = generator(state, control)
control = key
if key == nil then break end
```

终止条件只检查 `nil`，不能使用普通真值判断，因为 `false` 也可以是合法 table key。`pairs` 返回 `(next, table, nil)`；`ipairs` 返回一个递增整数的辅助生成器，并在第一个 `nil` 数组项处停止。

这一版先用 `NativeFunction` 枚举承载三个教学内建，避免提前引入 v0.16 的完整宿主函数适配器。这个临时边界在 README 和后续迁移清单中都显式登记。

## Rust 知识映射

- `Vec<Value>` 表达可变长度返回值，切片表达固定调用窗口；
- `Option<usize>` 区分固定宽度与开放宽度；
- `FrameOutcome` 把普通返回和尾调用变成显式状态转换；
- `checked_add`、`try_from` 和 255 寄存器上限守住窗口算术；
- table 迭代把共享借用限制在一次 `next_pair` 调用内，避免跨 VM 回调持有 `RefCell` 借用。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v10-multi-return-vararg/Cargo.toml -- `
  --dump-bytecode book/code/v10-multi-return-vararg/examples/demo.lua
cargo test --manifest-path book/code/v10-multi-return-vararg/Cargo.toml --workspace
```

示例最终输出应为：

```text
type=number value=1060
```

测试矩阵覆盖：

- 赋值、参数、return、括号和 table 构造器中的展开/折叠；
- vararg 截取、补 `nil`、转发，以及函数外误用；
- 5000 层尾递归与非尾递归深度限制；
- `pairs`、`ipairs`、`next` 和 Lua 函数生成器；
- 非 table 迭代参数与无效 `next` key 的错误。

## 本版本暂不支持

- 嵌套函数仍不能捕获外层 local；`local function` 的自引用也尚未成为 upvalue；
- 没有 metatable，`pairs`/`ipairs` 不会查询元方法；
- 三个迭代器是教学内建，还不是 v0.16 的标准库注册模型；
- 没有 GC，table 环仍会被 `Rc` 强引用保留；
- 不支持协程跨 `yield` 保存开放结果窗口。

这些限制没有隐藏实现。下一版会把“函数只保存 Proto”迁移为 closure，并让多个 closure 共享 open/closed upvalue。

## 从 v0.9 迁移了什么

- 删除：`execute_proto -> Value` 的单结果假设，以及“CALL 的 B/C 只是参数起点/数量”的旧解释；
- 重构：执行帧返回 `Vec<Value>`，CALL/RETURN 统一使用数量加一和 `0` 开放哨兵；
- 新增：`Proto::is_vararg`、`VARARG`、`TAILCALL`、`JUMPIFNIL`、`SETLISTOPEN` 与运行时 `top`；
- 新增：generic-for AST/Parser/Codegen，以及 table 的 `next_pair` 迭代边界；
- 调整：v0.9 的“无限尾递归触发调用深度”测试改为非尾递归，因为真正的尾调用不会创建新 Lua 帧。

一个更直观的替代方案，是让每条指令都在寄存器旁携带 `Vec<Value>`。它会简化首次实现，却把参数展开、table 末项和 return 都变成额外复制。本版采用寄存器窗口加 `top`，因为它能逐步映射到 Lua 5.1 的调用约定。另一个替代方案是用 Rust 递归执行 `TAILCALL`；5000 层验收会直接暴露它仍在消耗宿主栈。

## 章末练习

1. 给 `execute_multi` 增加“函数没有显式 return 时返回空列表”的测试，并说明 `execute` 为什么仍显示 `nil`。
2. 写出 `a, b, c = 1, (values())` 与 `a, b, c = 1, values()` 的对照测试。
3. 实现一个纯 Lua 的倒序数组迭代器，并用两个循环变量消费它。
4. 把尾递归次数从 5000 提高到 20000，分别观察 Debug/Release 的耗时；结果不应出现调用深度或 Rust 栈溢出。
5. 修改 `pairs` 测试，使 table 含有 `false` key，证明 generic for 只能以 `nil` 而不是真值作为终止条件。

## 与成熟实现和专题章节的对应

- 当前成熟实现中的调用栈、结果数量和 vararg 逻辑，是本版协议的最终映射目标；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可用于深入理解 dispatch loop；
- [用测试守住兼容性](../ch05-engineering/testing.md) 解释为什么值列表必须用矩阵而不是零散样例验证；
- 下一 checkpoint：[v0.11 · Closure 与 Upvalue](v11-closure-upvalue.md)。
