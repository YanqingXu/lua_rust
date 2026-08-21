# v0.3：Parser 与最小 AST

<div class="lesson-meta"><span>Checkpoint 4 / 20</span><span>Pratt parser</span><span>递归 enum</span></div>

## 本版本增量

v0.2 产生线性 Token 流。v0.3 的唯一核心增量是将它变成带源码位置的递归 AST，支持字面量、名字、括号、一元与二元表达式。

本版本仍然直接执行 AST，但求值器只支持数值算术、`not` 和字符串长度。比较、拼接和短路已能被解析，会在 v0.4 实现运行时语义。这个有意的缺口让 Parser 保持为唯一主题。

## AST 与 Rust 所有权

一个二元表达式包含两个子表达式，如果直接嵌入会导致类型大小无穷递归。`Box<Expr>` 用固定大小的指针打破这个循环，同时表达 AST 对子节点的唯一所有权。

```rust
{{#include ../../code/v03-parser/src/lib.rs:ast}}
```

`Result<Expr, ParseError>` 使 malformed input 不会靠 panic 退出；每个错误保留当前 Token 的行列。

## 优先级爬升

Pratt parser 将每个中缀运算符表示为左/右 binding power。左结合运算符的右 binding power 更高，右结合的幂运算和拼接则使用相同 binding power。

```rust
{{#include ../../code/v03-parser/src/lib.rs:pratt_parser}}
```

Lua 有两个重要边界：

- `2 ^ 3 ^ 2` 是 `2 ^ (3 ^ 2)`；
- `-2 ^ 2` 是 `-(2 ^ 2)`，但 `2 ^ -2` 也必须合法。

可选实现是为每一层优先级写一个函数。它容易初步理解，但右结合和幂/一元关系会分散在多个函数中。Pratt parser 让运算符表成为单一真相源。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v03-parser/Cargo.toml -- `
  book/code/v03-parser/examples/demo.lua
```

输出会展开 AST，其核心形状是 `Add(1, Multiply(2, 3))`，并以下列结果结束：

```text
result=7
```

```powershell
cargo test --manifest-path book/code/v03-parser/Cargo.toml
```

验收测试必须锁定算术优先级、幂与拼接的右结合、幂/一元关系以及 malformed input 的精确行列。

## 本版本暂不支持

- 比较、逻辑短路和拼接的求值；
- 定义名字、赋值和作用域；
- 语句、Table、函数和 Bytecode。

## 练习、迁移与深入

1. 手工画出 `not a and b or c` 的 AST，再与 Debug 输出比较。
2. 为 `(1 + 2` 增加错误位置测试。
3. 故意将幂改为左结合，确认哪个测试会报警。

从 v0.2 迁移时新增 AST、Parser 与结构化语法错误，CLI 从 Token dump 变为 AST/result。v0.4 不改变 AST 核心形状，只完成求值语义。成熟实现的对应阅读材料是 [Parser 与 AST](../ch03-compiler/parser.md)。
