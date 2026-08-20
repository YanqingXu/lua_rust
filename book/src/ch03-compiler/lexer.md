# 04 · Lexer：从字节到 Token

<div class="lesson-meta"><span>预计 25 分钟</span><span>Rust：生命周期 / 切片 / 状态机</span><span>编译器</span></div>

## Lexer 解决什么问题

Parser 不想逐个处理空格、注释和引号。Lexer 先把连续字节分成带类别与位置的 Token：

```lua
local answer = 40 + 2
```

可以被看成：

```text
Local · Name("answer") · Assign · Number(40) · Plus · Number(2) · Eos
```

Token 类型本身是一个拥有稳定判别值的 Rust enum：

```rust
{{#include ../../../crates/lua_compiler/src/token.rs:lua_token_type}}
```

单字符 Token 直接使用 ASCII 值，多字符运算符、关键字和字面量从 257 开始。这让调试与表驱动逻辑可以复用明确编号。

## 生命周期表达“Lexer 不拥有源码”

```rust
{{#include ../../../crates/lua_compiler/src/lexer.rs:lua_lexer_state}}
```

`Lexer<'source>` 借用一段 `&'source [u8]`。`_source` 是完整输入的生命周期锚点，`rest` 是尚未扫描的窗口。每前进一个字节，只需把 `rest` 换成更短的切片，不需要复制剩余源码。

生命周期参数表达了一个简单事实：Lexer 不能比被借用的源代码活得更久。Rust 把这个规则放在编译期验证，而不是等悬空指针出现。

## 为什么入口是 bytes

与上一章相同，Lua 源码里的字符串字面量和长字符串可能包含任意字节。`Lexer::new(&str)` 是方便入口，规范入口则是 `from_bytes(&[u8])`。扫描关键字和标点只需要识别 ASCII，字符串内容可以原样保留。

## 一个 Token 的预读

递归下降 Parser 经常需要先看下一个 Token 才能决定走哪条语法分支。Lexer 用 `Option<Token>` 保存一格预读缓存：

```rust
{{#include ../../../crates/lua_compiler/src/lexer.rs:lua_lexer_lookahead}}
```

这里有一个小而典型的所有权动作：

- `peek_token` 克隆结果，并把原值留在缓存中；
- `next_token` 使用 `Option::take()` 把值移出，同时把缓存恢复为 `None`。

`take` 避免了手写“先读取、再清空”时的临时非法状态。

## 扫描器就是显式状态机

Lexer 维护 `rest`、`line`、`column` 与 `lookahead`。扫描时按首字节选择分支：跳过空白、识别数字、读取名字、解析引号，最终生成 Token。错误也会带上扫描开始位置，而不是只给出“非法字符”。

对于这个问题，显式字段比抽象的通用解析框架更有教学价值：你可以逐步观察每个输入字节怎样改变状态。

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>手工写出 <code>local x = "a" .. "b"</code> 的 Token 序列。</li>
    <li>在 lexer 测试中为 Windows 换行 <code>\r\n</code> 增加一个位置断言。</li>
    <li>解释 <code>rest: &'source [u8]</code> 与 <code>Vec&lt;u8&gt;</code> 在所有权和扫描成本上的区别。</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>Lexer 借用源码、移动切片窗口并产出自有 Token；生命周期防止它越过输入存活，Option 缓存支持 Parser 的单 Token 预读。</p>
</div>

继续探索：[`lexer.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_compiler/src/lexer.rs) · [`token.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_compiler/src/token.rs)
