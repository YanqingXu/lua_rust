# v0.5：Bytecode 与最小寄存器 VM

<div class="lesson-meta"><span>Checkpoint 6 / 20</span><span>4-crate workspace</span><span>32-bit 指令</span></div>

## 架构转折点

v0.4 由 Rust 递归函数直接遍历 AST。v0.5 的唯一核心增量是把执行路径迁移为：

<div class="pipeline">
  <span>AST</span><b>→</b><span>Codegen</span><b>→</b><span>Proto</span><b>→</b><span>Register VM</span><b>→</b><span>Value</span>
</div>

这次迁移不增加新的 Lua 表达式能力。v0.4 的语义矩阵必须原样通过，所以测试能告诉我们：实现机制已换掉，语言合同没有被换掉。

## 为什么现在才拆 workspace

v0.0–v0.4 中，拆 crate 只会增加 Cargo 概念，却没有稳定的边界可教。现在数据流已自然分成四层：

| crate | 责任 | 依赖规则 |
|---|---|---|
| `lua_core` | `Value`、`Instruction`、`Opcode`、`Proto` | 不依赖编译器或 VM |
| `lua_compiler` | Lexer、Parser、AST、Codegen | 只产生 `lua_core` 类型 |
| `lua_vm` | 寄存器、dispatch loop、运行时错误 | 只消费 `Proto` |
| `lua_app` | 文件/stdin、反汇编选项和终端合同 | 组合其他三层 |

快照根目录同时是 `lua_app` package 和 workspace root，因此仍然保留统一的 `src/`、`tests/` 与 `examples/` 入口。

## 32 位指令格式

指令沿用 Lua 5.1 容易对照的布局：6 位 opcode、8 位 A、9 位 B 和 9 位 C。ABx 模式将 B/C 合并为 18 位常量索引，AsBx 再使用偏移编码表示有符号跳转。

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_core/src/lib.rs:instruction}}
```

Rust 学习点包括位移、掩码、`#[repr(u8)]`、`TryFrom<u8>` 与显式范围检查。编码/解码往返测试使位布局变成可执行合同。

`Proto` 是一段可执行单元，当前保存常量表、指令、行号表和最大寄存器数：

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_core/src/lib.rs:proto}}
```

可选方案是让每个指令直接保存一个 Rust enum 与完整操作数。它更容易调试，但无法教授真实字节码的空间合同，也难以在 v1.0 与 Lua 5.1 的 38 个 opcode 对照。本版本从开始就使用 32 位编码，同时用反汇编器恢复可读性。

## Codegen 与寄存器分配

每个字面量申请一个寄存器并生成 `LOADK`。一元运算就地覆盖输入寄存器；二元运算读取 B/C 并把结果写回 A。v0.5 使用只增不减的简单分配器，优先让寄存器流可见；后续引入局部变量时再学习释放与复用。

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_compiler/src/lib.rs:codegen}}
```

`and/or` 的右侧仍然会被编译，但运行时由 `JumpIfFalse`/`JumpIfTrue` 跳过。Codegen 先放入 offset 为 0 的占位指令，编译右侧后再回填目标。这是 v0.7 复杂控制流 patch list 的最小预演。

## Dispatch loop

VM 将 `max_stack_size` 转成寄存器数组，用 program counter 顺序取指、解码、执行。所有寄存器、常量与跳转访问都经过边界检查；恶意或损坏的 `Proto` 返回 `VmError`，不会导致 Rust panic。

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_vm/src/lib.rs:dispatch}}
```

VM 也带有指令数上限。当前 Codegen 只生成前向跳转，但这个限制从一开始就建立了宿主可以终止异常字节码的边界。

## 运行与预期输出

```powershell
cargo run --manifest-path book/code/v05-bytecode-vm/Cargo.toml -- `
  --dump-bytecode book/code/v05-bytecode-vm/examples/demo.lua
```

反汇编中可以看到两次短路跳转、一次乘法和一次拼接：

```text
0001 [  1] JumpIfFalse  0 +2
0004 [  1] JumpIfTrue   0 +6
0008 [  1] Mul          3 3 4
0009 [  1] Concat       2 2 3
0011 [  1] Return       0 0
type=string value="answer=42"
```

## 测试与验收

```powershell
cargo test --manifest-path book/code/v05-bytecode-vm/Cargo.toml --workspace
```

验收分为四层：

1. `lua_core` 的 ABC/AsBx 编码解码往返；
2. `lua_compiler` 的优先级指令顺序与 Proto shape；
3. `lua_vm` 对非法寄存器/常量的拒绝；
4. `lua_app` 的 v0.4 语义回归、短路和反汇编合同。

统一质量门还会执行 `cargo fmt --check` 和 `cargo clippy --workspace --all-targets -- -D warnings`。

## 本版本暂不支持

- 任何语句、变量、赋值与作用域；
- Table、函数、调用帧和多返回；
- Lua 5.1 全部 38 个 opcode；当前只有表达式 VM 的最小集合；
- binary chunk 读写、跨版本字节码 ABI 与优化寄存器分配。

## 练习与迁移

1. 手工跟踪示例在 pc 0–11 期间的寄存器变化，标出哪些指令被跳过。
2. 将一条指令的常量索引改为越界值，确认 VM 返回带 pc/行号的错误。
3. 增加 `true and 1 or 2` 的反汇编快照，计算两个跳转的 target。
4. 实验性释放右子树寄存器，观察 `max_stack_size` 如何下降。

从 v0.4 迁移时，删除 AST evaluator，新增 `Instruction/Opcode/Proto`、Codegen、反汇编器与 VM dispatch，并将单 crate 拆成四个责任边界。v0.6 将在此基础上新增 global/local、多重赋值、作用域栈与局部寄存器复用。成熟实现的对应阅读材料是 [Bytecode](../ch03-compiler/bytecode.md) 与 [寄存器 VM](../ch04-vm/register-vm.md)。
