# v0.2：Lexer 与 Token

<div class="lesson-meta"><span>Checkpoint 3 / 20</span><span>字节状态机</span><span>源码位置</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>让借用的源码沿状态机前进</strong>
  <p>前置：v0.1 的字节语义、enum 和 Result。新知识：生命周期参数、切片窗口、cursor
  不变量、单字节 lookahead。完成后你应能手工跟踪 offset/line/column，并解释 Token
  为什么拥有转义后的字节。</p>
</div>

本章按六个微课阅读：Token 模型 → 源码借用 → cursor → 名字/数字 → 字符串/注释 →
位置错误。一次只跟踪一种状态，不要试图第一遍读完 500 多行 `lib.rs`。

<a id="learning-predict"></a>
## 先预测再运行

把 `1..2` 写在纸上，先预测 Token 数量和每个 Token 的范围。正确结果必须是 number `1`、
concat `..`、number `2`、EOF；如果 Lexer 贪婪地把 `1..2` 当作数字，Parser 将失去恢复
语言结构的机会。

再预测 `[==[a]=]b]==]` 的字符串内容：中间的 `]=]` 层级不足，不能结束 level 2 长字符串。

## 本版本增量

v0.1 只能把整份输入当成一个字面量。v0.2 将它替换为真正的 Lua 5.1 Lexer：从 `&[u8]` 顺序产生关键字、名字、数字、长短字符串、运算符与 EOF Token，同时跳过空白和注释。每个 Token 都保存 byte offset、行与列。

这一版的输出是 Token 流，不会执行 `return`、`local` 或任何运算。

## Lua 语义与 Rust 知识

- 长字符串和长注释的 `=` 层级必须成对；
- 长字符串开头紧跟的第一个换行不属于内容；
- 短字符串的十进制转义可以生成 `0x00..0xff` 任意字节；
- `1..2` 必须分成数字 `1`、拼接 `..`、数字 `2`，而不是一个错误数字。

Lexer 借此引入切片借用、生命周期、显式 cursor、单字节预读和循环状态机。`Lexer<'source>` 借用输入，Token 则拥有名字与字符串的语义字节。这使 Parser 不依赖 Lexer 的内部缓冲区。

<a id="learning-code-map"></a>
## 代码阅读地图

1. `TokenKind` 与 `Token`：语义值和位置分别保存；
2. `Lexer<'source>::new`：只借用 `&'source [u8]`，不复制整份源码；
3. `current/peek/bump`：所有分支共用的 cursor 原语；
4. `next_token`：清理 trivia 后只负责顶层分派；
5. `lex_name_or_keyword/lex_number`：在连续字节上推进；
6. `lex_short_string/lex_long_string`：创建拥有的解码结果；
7. `error`：在失败发生处冻结当前位置。

关键不变量是 `offset` 永远指向下一个尚未消费的字节，`line/column` 描述同一位置。
任何分支都只能通过 `bump/advance` 移动 cursor；如果直接修改 offset，行列将与字节位置
分裂。生命周期 `'source` 则表示 Lexer 不能比输入切片活得更久，但 Token 中解码后的
字符串拥有自己的 `Box<[u8]>`，不受 Lexer 生命周期约束。

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

把 `malformed_input_reports_its_location` 当作规格阅读：测试不是只要求“出错”，还要求
错误发生在未终止输入或非法字节的准确位置。修改 cursor 代码后，应先运行这一小组，
再运行全量测试。

## 本版本暂不支持

- AST 和任何语法结构；
- 表达式求值、变量与语句；
- Lua 数字的平台 C `strtod` 完全细节。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>手工跟踪 <code>[==[a]=]b]==]</code> 的 cursor，解释中间的 <code>]=]</code> 为什么
  不会结束字符串。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为 CR、LF、CRLF 增加位置测试，明确每种换行后的 offset、line、column 合同；只修改
  本快照并运行 <code>cargo test</code>。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>临时让数字扫描无条件吞掉第二个点，运行 <code>1..2</code> 相关测试，解释失败为什么发生
  在 Lexer 而不是 Parser，然后恢复 lookahead 条件。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出 `Lexer<'source>`、输入切片和拥有型 Token 的生命期关系；
- 能逐字节更新 `offset/line/column`；
- 能说明 `current` 与 `peek` 返回 `Option<u8>` 如何避免越界 panic。

## 从本版迁往 v0.3

从 v0.1 迁移时，删除顶层 `evaluate_literal`，把字面量解码移入 Lexer，新增 `Position`、`Token` 和预读逻辑。v0.3 会保留这些词法规则，但将 Token 流交给 Parser。成熟实现对应 [Lexer 专题](../ch03-compiler/lexer.md)。
