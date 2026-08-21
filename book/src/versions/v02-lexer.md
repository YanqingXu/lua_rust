# v0.2：Lexer 与 Token

<div class="lesson-meta"><span>Checkpoint 3 / 20</span><span>字节状态机</span><span>源码位置</span></div>

## 本版本增量

v0.1 只能把整份输入当成一个字面量。v0.2 将它替换为真正的 Lua 5.1 Lexer：从 `&[u8]` 顺序产生关键字、名字、数字、长短字符串、运算符与 EOF Token，同时跳过空白和注释。每个 Token 都保存 byte offset、行与列。

这一版的输出是 Token 流，不会执行 `return`、`local` 或任何运算。

## Lua 语义与 Rust 知识

- 长字符串和长注释的 `=` 层级必须成对；
- 长字符串开头紧跟的第一个换行不属于内容；
- 短字符串的十进制转义可以生成 `0x00..0xff` 任意字节；
- `1..2` 必须分成数字 `1`、拼接 `..`、数字 `2`，而不是一个错误数字。

Lexer 借此引入切片借用、生命周期、显式 cursor、单字节预读和循环状态机。`Lexer<'source>` 借用输入，Token 则拥有名字与字符串的语义字节。这使 Parser 不依赖 Lexer 的内部缓冲区。

## 核心实现

`next_token` 先清理 trivia，再根据当前字节进入名字、数字、字符串或固定符号分支：

```rust
{{#include ../../code/v02-lexer/src/lib.rs:next_token}}
```

长括号不能只用“查找下一个 `]]`”实现，因为 `[==[...]==]` 允许内容包含较短的结束形状。实现先记录 level，然后只接受完全匹配的闭合符：

```rust
{{#include ../../code/v02-lexer/src/lib.rs:long_string}}
```

可选设计是让 Token 只保存指向源码的 range。它能减少名字分配，但转义后字符串并不是原始源码的连续切片，还会把源文件生命周期扩散到整棵 AST。教学快照选择让 Token 拥有语义值。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v02-lexer/Cargo.toml -- `
  book/code/v02-lexer/examples/demo.lua
```

输出为带位置的 Token 流，其中应包含：

```text
2:1 local
2:7 name(message)
2:15 equal
2:17 string("hello\nLua")
4:1 return
```

实际输出使用 tab 分隔位置与 Token，并以 `<eof>` 结束。

```powershell
cargo test --manifest-path book/code/v02-lexer/Cargo.toml
```

验收要求包括关键字/名字区分、`1..2`、十六进制数、十进制转义、长字符串换行、注释与非法字节位置。

## 本版本暂不支持

- AST 和任何语法结构；
- 表达式求值、变量与语句；
- Lua 数字的平台 C `strtod` 完全细节。

## 练习、迁移与深入

1. 增加 `[==[a]=]b]==]` 的测试，解释为什么中间的 `]=]` 不会结束字符串。
2. 为 Windows `\r\n` 增加位置测试，再决定列数合同。
3. 把一个未终止短字符串改成合法输入，比较错误位置与 Token 位置。

从 v0.1 迁移时，删除顶层 `evaluate_literal`，把字面量解码移入 Lexer，新增 `Position`、`Token` 和预读逻辑。v0.3 会保留这些词法规则，但将 Token 流交给 Parser。成熟实现对应 [Lexer 专题](../ch03-compiler/lexer.md)。
