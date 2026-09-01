# v0.5：Bytecode 与最小寄存器 VM

<div class="lesson-meta"><span>Checkpoint 6 / 20</span><span>4-crate workspace</span><span>32-bit 指令</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把直接求值拆成编译与执行</strong>
  <p>前置：v0.4 的 AST 和语义 oracle。新知识：workspace 依赖边界、位域编码、常量表、
  寄存器分配、跳转回填和 dispatch 状态机。完成后你应能从 AST 手推 Proto，再逐条更新
  pc 与寄存器。</p>
</div>

本版信息量较大，请按五个微课阅读：crate 边界 → 指令位域 → Proto → Codegen → VM。
每完成一节就用反汇编结果验证，不要把四个 crate 当成一个大文件阅读。

<a id="learning-predict"></a>
## 先预测再运行

对表达式 `40 + 2`，先画出你认为最少需要的常量、寄存器和指令。再运行
`--dump-bytecode` 对照。重点不是猜中 opcode 名，而是回答：常量放在哪里、运算中间值
放在哪里、最终结果由哪条指令交还调用者。

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

依赖箭头只能指向更稳定的数据层：app 可以组合 compiler 和 VM；VM 可以消费 core 的
Proto；core 不能反过来调用 app。这个规则避免 `Value` 为了打印错误而依赖 CLI，也让
compiler 与 VM 能分别单测。crate 边界不是文件整理，而是编译器强制执行的架构合同。

<a id="learning-code-map"></a>
## 代码阅读地图

1. `lua_core::Opcode/Instruction`：把操作和操作数编码为 `u32`；
2. `lua_core::Proto`：拥有常量、代码、行号和最大寄存器数；
3. `lua_compiler::compile`：Lexer → Parser → Codegen；
4. `Codegen::expression/register/constant/emit`：把树变成线性指令；
5. `patch_jump`：表达式编译完后回填短路目标；
6. `Vm::execute`：维护 pc 和寄存器数组；
7. `lua_app::execute`：只负责组装 compiler 与 VM 的错误边界；
8. `tests/pipeline.rs`：证明 v0.4 语义在迁移后不变。

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

叶节点和一元节点先展示最小的“申请寄存器 → 发指令 → 返回结果位置”协议：

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_compiler/src/lib.rs:codegen_leaf_unary}}
```

短路表达式需要先发占位跳转，编译右侧后再回填：

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_compiler/src/lib.rs:codegen_short_circuit}}
```

普通二元节点则依次获得左右寄存器，并把结果覆盖到左寄存器：

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_compiler/src/lib.rs:codegen_eager_binary}}
```

`and/or` 的右侧仍然会被编译，但运行时由 `JumpIfFalse`/`JumpIfTrue` 跳过。Codegen 先放入 offset 为 0 的占位指令，编译右侧后再回填目标。这是 v0.7 复杂控制流 patch list 的最小预演。

## Dispatch loop

VM 将 `max_stack_size` 转成寄存器数组，用 program counter 顺序取指、解码、执行。所有寄存器、常量与跳转访问都经过边界检查；恶意或损坏的 `Proto` 返回 `VmError`，不会导致 Rust panic。

先看 VM 如何验证 Proto、分配寄存器并安全取指：

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_vm/src/lib.rs:dispatch_setup_fetch}}
```

值操作只读写寄存器，控制操作还会改变 `next_pc` 或直接返回：

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_vm/src/lib.rs:dispatch_value_ops}}
```

```rust
{{#include ../../code/v05-bytecode-vm/crates/lua_vm/src/lib.rs:dispatch_control_ops}}
```

VM 也带有指令数上限。当前 Codegen 只生成前向跳转，但这个限制从一开始就建立了宿主可以终止异常字节码的边界。

以 `40 + 2` 为例，可以按下面的状态表阅读 dispatch；实际寄存器编号以反汇编为准：

| pc | 指令 | 执行前 | 执行后 |
|---:|---|---|---|
| 0 | `LOADK R0 K0` | `R0=nil` | `R0=40` |
| 1 | `LOADK R1 K1` | `R1=nil` | `R1=2` |
| 2 | `ADD R0 R0 R1` | `R0=40,R1=2` | `R0=42` |
| 3 | `RETURN R0` | `R0=42` | 返回 `42` |

`Proto` 不变、寄存器可变、pc 单调前进或被跳转改写，是本版 VM 的三个核心状态边界。
所有索引访问仍返回 `Result`，损坏 Proto 只能产生 `VmError`，不能越界 panic。

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

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>手工跟踪示例在 pc 0–11 期间的寄存器变化，标出短路跳过的指令和跳转 target。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>给 <code>true and 1 or 2</code> 增加反汇编快照，并在期望文本中锁定两个跳转目标。
  要求 v0.4 迁移而来的全部语义测试仍然通过。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>将一条指令的常量索引改成越界值，跟踪它在哪个边界返回带 pc/行号的 VmError，确认
  测试进程没有 panic。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出四个 crate 的依赖方向并解释为什么 core 位于底层；
- 能从一棵二元 AST 手推常量表、寄存器和指令；
- 能按 pc 逐条更新寄存器并指出所有越界检查；
- 能说明 v0.4 测试如何证明“机制改变但语义未改变”。

## 从本版迁往 v0.6

从 v0.4 迁移时，删除 AST evaluator，新增 `Instruction/Opcode/Proto`、Codegen、反汇编器与 VM dispatch，并将单 crate 拆成四个责任边界。v0.6 将在此基础上新增 global/local、多重赋值、作用域栈与局部寄存器复用。成熟实现的对应阅读材料是 [Bytecode](../ch03-compiler/bytecode.md) 与 [寄存器 VM](../ch04-vm/register-vm.md)。
