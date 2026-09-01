# v0.4：完整表达式语义

<div class="lesson-meta"><span>Checkpoint 5 / 20</span><span>运算分发</span><span>短路求值</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把 AST shape 变成 Lua 行为</strong>
  <p>前置：v0.3 的递归 AST 与 Result。新知识：按值/按引用匹配、短路控制流、类型分发、
  临时值移动和结构化运行时错误。完成后你应能解释为什么 and/or 不能走普通二元路径。</p>
</div>

<a id="learning-predict"></a>
## 先预测再运行

逐个写下结果或错误：`0 or missing`、`nil and missing`、`1 == "1"`、`-3 % 2`、
`#"A\000\255"`。前两个表达式还要标明右侧是否执行。预测不仅包含值，也包含求值顺序。

## 本版本增量

v0.3 已经能正确构建表达式 AST，但只执行一小部分节点。v0.4 的唯一核心增量是完成 Lua 表达式语义：数值算术、比较、`and/or`、拼接、长度和类型错误。此时仍直接执行 AST，v0.5 才会用字节码和 VM 替换它。

## 最容易被宿主语言带偏的语义

- `0` 与 `""` 在 Lua 中是真；
- `a and b` 返回 `a` 或 `b` 本身，不会强制转为 boolean；
- `a or b` 也返回原值，并且右侧可能完全不求值；
- `%` 使用 floor division 定义，所以 `-3 % 2 == 1`；
- 相等比较不隐式转换类型，`1 == "1"` 为假；
- 字符串长度是字节数，`#"A\000\255" == 3`。

## 求值顺序就是语义

`and/or` 不能与普通二元运算符共用“先求左右两侧”的通用路径。它们必须在求右侧之前检查左值：

```rust
{{#include ../../code/v04-expressions/src/lib.rs:evaluator}}
```

其他运算符先得到两个 Value，再进入类型分发。先读比较分支，注意跨类型如何返回错误或
不相等，而不是强制转换：

```rust
{{#include ../../code/v04-expressions/src/lib.rs:compare_operators}}
```

拼接消费两个临时值，算术则借用匹配得到的两个 `f64`：

```rust
{{#include ../../code/v04-expressions/src/lib.rs:concat_operator}}
```

```rust
{{#include ../../code/v04-expressions/src/lib.rs:arithmetic_operators}}
```

Rust 的所有权会在此时影响 API：拼接消费两个临时 Value，可以直接复用其字节 buffer；比较只借用 Value，避免不必要的克隆。`Result` 使类型错误通过递归求值树自然向上传播。

<a id="learning-code-map"></a>
## 代码阅读地图

1. `execute`：串起 parse 与 evaluate，并保留错误类别；
2. `evaluate(&Expr)`：递归解释节点，先处理短路分支；
3. `evaluate_eager_binary`：只接收已经求值的左右 Value；
4. `Value::into_concat_bytes(self)`：消费临时 Value，允许移动字节；
5. `compare(&Value, &Value)`：只读比较，不夺走调用者的值；
6. `type_error/runtime_error`：附加运算符位置；
7. 语义矩阵测试：成为 v0.5 VM 迁移时的 oracle。

这里最值得学习的是函数签名。`compare` 接收引用，因为比较后左右值可能仍被使用；拼接
helper 接收拥有的 Value，因为两边都是递归求值得到的临时结果，可以移动内部 buffer。
先决定值是否还需要使用，再决定借用还是消费，比看到借用错误后盲目 `clone()` 更可靠。

## 设计取舍

可以在 Parser 里对纯字面量进行常量折叠，例如直接把 `1 + 2` 变成 `3`。本版本不做这件事：我们需要让每个运算符都经过同一求值路径，先把语义测试锁定。等 v0.5 之后，常量折叠才能作为不改变语义的编译器优化。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v04-expressions/Cargo.toml -- `
  book/code/v04-expressions/examples/demo.lua
```

示例中的 `missing` 未定义，但它处于不应求值的短路分支。预期输出：

```text
type=string value="answer=42"
```

```powershell
cargo test --manifest-path book/code/v04-expressions/Cargo.toml
```

验收矩阵覆盖真值、返回原值的 `and/or`、不求值的右侧、负数取模、幂优先级、字节长度、数字拼接、字符串比较、跨类型相等和运算符错误位置。

先单独运行 `and_or_return_values_and_short_circuit`。测试中未定义名字故意放在不应执行的
右侧：如果通用二元路径先递归求两个操作数，它会立刻暴露错误。这个失败比查看实现更能
证明短路是控制流，而不是一种 boolean 运算。

## 本版本暂不支持

- 语句、局部/全局变量和赋值；
- Table、函数、多返回与元方法；
- Bytecode、VM 和任何性能优化；
- Lua 元方法中的数字/字符串转换扩展。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>解释为什么 <code>-3 % 2</code> 不能盲目照搬宿主语言的余数运算，并写出 Lua floor
  division 对应的等式。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为 <code>nil and missing</code>、<code>0 or missing</code>、<code>"" or missing</code>
  写表驱动测试，同时断言返回原值和是否触发右侧错误。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>暂时把 and/or 移入 eager binary 路径，运行短路测试，沿错误位置说明哪次递归求值
  违反了 Lua 合同，然后恢复专用分支。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出短路表达式实际访问的 AST 节点；
- 能根据 helper 是否消费参数决定使用 `Value` 还是 `&Value`；
- 能说明本版测试为什么可以充当下一版 VM 的语义 oracle。

## 从本版迁往 v0.5

从 v0.3 迁移时保留 Lexer、AST 与优先级表，用完整 Value 分发替换最小 evaluator，新增类型化运行时错误。v0.5 将删除 AST 直接执行路径，新增 Codegen、Proto、寄存器和 dispatch loop，但本章的整套语义测试必须保持通过。对应的成熟实现将在 [Bytecode](../ch03-compiler/bytecode.md) 与 [寄存器 VM](../ch04-vm/register-vm.md) 中继续对照。
