# 05 · Parser：用 AST 表达语法

<div class="lesson-meta"><span>预计 25 分钟</span><span>Rust：Box / 递归 enum / Result</span><span>编译器</span></div>

## 从平坦 Token 恢复结构

Token 序列只保留顺序，不直接表达优先级。下面两个表达式使用相同种类的 Token，却有不同结构：

```lua
1 + 2 * 3
(1 + 2) * 3
```

Parser 的工作是根据语法和优先级构造 AST。第一段的根节点是加法，右孩子是乘法；第二段的根节点是乘法，左孩子是带括号的加法。

## 递归数据需要间接层

项目把所有表达式节点收进 `Expr`：

```rust
{{#include ../../../crates/lua_compiler/src/ast/expr.rs:lua_expr_ast}}
```

`BinaryExpr` 中继续包含 `Box<Expr>`。为什么需要 `Box`？如果 enum 直接内嵌另一个 `Expr`，编译器无法算出类型大小：一个表达式可能包含表达式，后者又包含表达式，无限展开。

`Box<T>` 在栈上只保存固定大小的指针，真正节点在堆上，因此递归类型获得了确定大小。它同时表达独占所有权：父节点拥有子节点，AST 被释放时整棵树会自动递归释放。

## enum 让非法节点组合更难出现

每个 variant 只携带该节点需要的数据。例如 Number 不会意外带函数参数，Call 必须拥有被调用表达式与参数列表。与“一个大 struct + type 字段 + 一堆 Option”相比，enum 避免了大量无意义状态。

访问节点时再次使用穷尽 match：

```rust
fn kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Number(_) => "number",
        Expr::Binary(_) => "binary",
        Expr::Call(_) => "call",
        // 其余分支也必须明确处理
        _ => "other",
    }
}
```

## 错误是一等返回值

语法错误来自用户输入，不应让编译器进程 panic。Parser 使用带行列信息的 `ParseError`，解析函数返回 `Result<T, ParseError>`，并通过 `?` 向上转交失败。

这种控制流很适合递归下降：每层只补充自己知道的上下文，最外层统一决定怎样展示错误。`Result` 让成功路径与失败路径都在函数签名中可见。

## 优先级：先解析更紧的操作

表达式解析通常接受“最低允许优先级”，读取左侧后不断查看下一个二元操作符。如果操作符优先级足够高，就递归解析右侧并组合为 `BinaryExpr`。乘法比加法绑定得更紧，因此自然落在更深的 AST 层级。

这是一种 Pratt / precedence-climbing 风格。重点不在名称，而在不变量：每次递归只消费属于当前优先级层的 Token。

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>为 <code>a + b * c</code> 画出三层 AST，并标出每个 Box 指向哪里。</li>
    <li>运行 <code>cargo test -p lua_compiler parser</code>，找到运算符优先级测试。</li>
    <li>输入 <code>local = 1</code>，观察错误是否同时包含行号与列号。</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>AST 用 enum 表达节点集合，用 Box 打断无限大小递归，用 Result 把用户语法错误变成可组合的普通控制流。</p>
</div>

继续探索：[`ast/expr.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_compiler/src/ast/expr.rs) · [`parser/`](https://github.com/YanqingXu/lua_rust/tree/main/crates/lua_compiler/src/parser)
