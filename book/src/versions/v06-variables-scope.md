# v0.6：变量、赋值与作用域

<div class="lesson-meta"><span>Checkpoint 7 / 20</span><span>statement AST</span><span>作用域栈</span></div>

## 本版本增量

v0.5 的 VM 只能执行一个无名字绑定的表达式。v0.6 的唯一核心增量是“脚本可以保存与更新状态”：Parser 新增 statement AST，Codegen 新增 global/local 名字解析、普通/多重赋值与 `do ... end` 作用域，VM 新增全局表。

从这一版开始，输入是 Lua chunk 而不是裸表达式：

```lua
local left, right = 10, 20
left, right = right, left
return left * 100 + right
```

## Lua 名字语义

- 未定义 global 的值是 `nil`，读取它不是运行时错误；
- 没有 initializer 的 local 以 `nil` 初始化；
- local 的 initializer 在新 local 可见之前求值，所以 `local x = x` 读取外层 `x`；
- 内层 block 可以遮蔽同名 local，离开 block 后恢复外层绑定；
- 多重赋值先求完全部右值，再从左到右写入，因此 `a,b=b,a` 能交换值；
- 右值不足时补 `nil`，多余右值仍会求值但不会写入。

## local 为什么是寄存器

局部名字在编译时就能解析，因此不需要运行时 HashMap 查找。每个活跃 local 占据一个稳定寄存器，临时表达式寄存器从 local 区间之后开始。读取 local 时先 `MOVE` 到临时寄存器，避免一元/二元指令覆盖变量本身。

作用域栈的每层记录进入时的 register base 与名字到寄存器的映射。离开 block 时弹出该层并恢复 base，内层 local 占据的寄存器就可以被下一个 block 复用。

```rust
{{#include ../../code/v06-variables-scope/crates/lua_compiler/src/lib.rs:bindings}}
```

这里不使用存储 `&str` 的符号表，因为 Lua 名字和字符串的底层输入仍然是字节。`HashMap<ByteString, u8>` 拥有 key，让 Lexer 的输入借用不会泄漏进 Codegen 生命周期。

## 并行赋值不是逐个赋值

Codegen 为所有右值分配临时寄存器，完整编译后才解析并写入左值。如果把求值与写入放在同一循环，`a,b=b,a` 在读取第二个右值时已经丢失旧 `a`。这个测试是赋值实现的核心证据。

local declaration 还有一个额外顺序要求：先在旧 scope 中编译全部 initializer，再把新名字插入当前 scope。

## global 为什么在 VM 中

global 是运行时状态，同一个 `Vm` 可以在多次 execute 之间保留它。`GETGLOBAL` 对不存在的 key 返回 `nil`；`SETGLOBAL` 写入 `nil` 时删除 key。指令使用 ABx 格式：A 是值寄存器，Bx 指向保存名字的字符串常量。

```rust
{{#include ../../code/v06-variables-scope/crates/lua_vm/src/lib.rs:globals}}
```

可选方案是把 global table 放在 `Proto`。那会混淆“不可变编译产物”与“每次执行的可变状态”，也使同一 Proto 无法在不同环境中运行。因此 global 明确属于 VM/Runtime。

## 运行与预期结果

```powershell
cargo run --manifest-path book/code/v06-variables-scope/Cargo.toml -- `
  --dump-bytecode book/code/v06-variables-scope/examples/demo.lua
```

反汇编中可以看到交换前先读入两个临时寄存器，也可以看到 block 内的 global 写入：

```text
0004 [  2] Move         2 1
0005 [  2] Move         3 0
0006 [  2] Move         0 2
0007 [  2] Move         1 3
0011 [  6] SetGlobal    3 3
type=number value=2010
```

## 测试与验收

```powershell
cargo test --manifest-path book/code/v06-variables-scope/Cargo.toml --workspace
```

验收覆盖：

- `a,b=b,a` 使用赋值前值；
- block 内层遮蔽不改变外层 local；
- initializer 中的同名名字解析为外层绑定；
- 未定义 global 与未初始化 local 均为 `nil`；
- global/local 使用 `GETGLOBAL/SETGLOBAL` 与 `MOVE` 两套存储；
- 第 256 个活跃寄存器产生结构化 `CompileError` 而不是整数溢出或 panic。

## 本版本暂不支持

- `if`、`while`、`repeat`、`for` 和 `break`；
- Table field 左值与其他复合赋值目标；
- 函数、调用和闭包；
- 多返回展开；`return a,b` 的完整宽度规则到 v0.10 实现。

## 练习与迁移

1. 为 `local x = x` 同时在“有外层 local”和“只有 global”情况下写测试。
2. 增加 `local a,b,c=1` 并验证 `b/c` 都为 `nil`。
3. 记录两个先后 block 的 `max_stack_size`，确认第一个 block 的 local 寄存器被复用。
4. 尝试把赋值改成“求一个写一个”，观察交换测试如何阻止这个回归。

从 v0.5 迁移时，裸表达式入口升级为 chunk/statement AST，新增 `Scope`、local binder、赋值右值缓冲、`GETGLOBAL/SETGLOBAL` 和 VM global table。v0.7 会保留这套名字与寄存器合同，再用跳转回填加入条件和循环。
