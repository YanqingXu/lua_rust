# v0.3：Parser 与最小 AST

<div class="lesson-meta"><span>Checkpoint 4 / 20</span><span>Pratt parser</span><span>递归 enum</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把平坦 Token 组合成递归结构</strong>
  <p>前置：v0.2 的 Token、cursor 与 Result。新知识：递归 enum、Box、Pratt binding power、
  递归错误传播。完成后你应能手画 AST，并说明每个 Box 的唯一 owner。</p>
</div>

<a id="learning-predict"></a>
## 先预测再运行

先为 `-2 ^ 3 ^ 2 + 4` 画树，不要计算数值。标出哪个运算符是根、两个 `^` 如何结合、
一元负号包住哪个子树。随后用 `--nocapture` 运行优先级测试核对结构。Parser 的正确性
首先是“树形正确”，不是“碰巧算出相同数字”。

## 本版本增量

v0.2 产生线性 Token 流。v0.3 的唯一核心增量是将它变成带源码位置的递归 AST，支持字面量、名字、括号、一元与二元表达式。

本版本仍然直接执行 AST，但求值器只支持数值算术、`not` 和字符串长度。比较、拼接和短路已能被解析，会在 v0.4 实现运行时语义。这个有意的缺口让 Parser 保持为唯一主题。

## AST 与 Rust 所有权

一个二元表达式包含两个子表达式，如果直接嵌入会导致类型大小无穷递归。`Box<Expr>` 用固定大小的指针打破这个循环，同时表达 AST 对子节点的唯一所有权。

```rust
{{#include ../../code/v03-parser/src/lib.rs:ast}}
```

`Result<Expr, ParseError>` 使 malformed input 不会靠 panic 退出；每个错误保留当前 Token 的行列。

<a id="learning-code-map"></a>
## 代码阅读地图

1. `ExprKind`：叶节点与递归节点的完整集合；
2. `Box<Expr>`：让递归字段具有固定指针大小，并表示唯一所有权；
3. `Parser::current/advance`：在拥有的 `Vec<Token>` 上移动索引；
4. `expression(minimum_binding_power)`：先读 prefix，再循环接中缀；
5. `infix_binding_power`：唯一的优先级与结合性表；
6. `ParseError`：冻结无法继续时的 Token 位置；
7. 优先级单测：直接检查 AST shape，而不是只检查最终结果。

如果把二元节点写成 `Binary { left: Expr, right: Expr }`，Rust 无法计算 `Expr` 的固定大小：
一个 Expr 内又完整包含 Expr，递归永不结束。`Box<Expr>` 把字段变成固定大小的指针；
子节点仍由父节点唯一拥有，AST 被释放时会递归释放，无需 GC。

## 优先级爬升

Pratt parser 将每个中缀运算符表示为左/右 binding power。左结合运算符的右 binding power 更高，右结合的幂运算和拼接则使用相同 binding power。

先看 prefix 阶段怎样把一个 Token 变成左侧表达式；此时递归调用只用于一元运算和括号：

```rust
{{#include ../../code/v03-parser/src/lib.rs:pratt_prefix}}
```

再看中缀循环。它只关心 binding power、递归得到的右树和重新组装的左树：

```rust
{{#include ../../code/v03-parser/src/lib.rs:pratt_infix}}
```

Lua 有两个重要边界：

- `2 ^ 3 ^ 2` 是 `2 ^ (3 ^ 2)`；
- `-2 ^ 2` 是 `-(2 ^ 2)`，但 `2 ^ -2` 也必须合法。

可选实现是为每一层优先级写一个函数。它容易初步理解，但右结合和幂/一元关系会分散在多个函数中。Pratt parser 让运算符表成为单一真相源。

手推 `1 + 2 * 3` 时，外层以最低 binding power 开始；读到 `+` 后解析右侧，但 `*` 的
binding power 更高，会先进入右子树。结果不是靠“乘法特殊分支”得到，而是由两个数字
比较驱动。这正是表驱动 Parser 比多层函数更容易验证的地方。

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

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>手工画出 <code>not a and b or c</code> 的 AST，并在每条边上标注父节点拥有哪个 Box。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为 <code>(1 + 2</code> 增加精确错误位置测试，并让失败信息指出缺失的右括号。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>故意将幂运算改为左结合，只运行 <code>power_and_concat_are_right_associative</code>，
  比较实际树和期望树，定位 binding power 中哪一个数字表达结合性。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能解释递归 enum 为什么需要间接层以及 `Box` 为什么不是共享所有权；
- 能手推 prefix、infix 和 minimum binding power 的变化；
- 能从错误 Token 反推 ParseError 的行列。

## 从本版迁往 v0.4

从 v0.2 迁移时新增 AST、Parser 与结构化语法错误，CLI 从 Token dump 变为 AST/result。v0.4 不改变 AST 核心形状，只完成求值语义。成熟实现的对应阅读材料是 [Parser 与 AST](../ch03-compiler/parser.md)。
