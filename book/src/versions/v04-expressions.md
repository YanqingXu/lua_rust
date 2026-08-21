# v0.4：完整表达式语义

<div class="lesson-meta"><span>Checkpoint 5 / 20</span><span>运算分发</span><span>短路求值</span></div>

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

其他运算符先得到两个 Value，再进入类型分发。运算符的源码位置会被带入运行时错误：

```rust
{{#include ../../code/v04-expressions/src/lib.rs:eager_operators}}
```

Rust 的所有权会在此时影响 API：拼接消费两个临时 Value，可以直接复用其字节 buffer；比较只借用 Value，避免不必要的克隆。`Result` 使类型错误通过递归求值树自然向上传播。

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

## 本版本暂不支持

- 语句、局部/全局变量和赋值；
- Table、函数、多返回与元方法；
- Bytecode、VM 和任何性能优化；
- Lua 元方法中的数字/字符串转换扩展。

## 练习、迁移与下一站

1. 为 `nil and missing`、`0 or missing`、`"" or missing` 分别写测试。
2. 解释为什么 `-3 % 2` 不能直接照搬某些宿主语言的余数运算。
3. 为 boolean 拼接增加错误测试，并检查行列是否指向 `..`。

从 v0.3 迁移时保留 Lexer、AST 与优先级表，用完整 Value 分发替换最小 evaluator，新增类型化运行时错误。v0.5 将删除 AST 直接执行路径，新增 Codegen、Proto、寄存器和 dispatch loop，但本章的整套语义测试必须保持通过。对应的成熟实现将在 [Bytecode](../ch03-compiler/bytecode.md) 与 [寄存器 VM](../ch04-vm/register-vm.md) 中继续对照。
