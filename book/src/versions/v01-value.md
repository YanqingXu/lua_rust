# v0.1：Lua Value 与字面量

<div class="lesson-meta"><span>Checkpoint 2 / 20</span><span>动态值</span><span>字节语义</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>用静态类型表示动态值</strong>
  <p>前置：v0.0 的 owner、借用和 <code>Result</code>。新知识：<code>enum</code>、穷尽
  <code>match</code>、newtype、<code>Box&lt;[u8]&gt;</code> 与 <code>Option</code>。完成后你应能
  解释 Value 的每个变体怎样保存数据，以及 Lua 字符串为什么不能用 Rust String。</p>
</div>

<a id="learning-predict"></a>
## 先预测再运行

先写下四个输入的类型、显示结果和真假性：`nil`、`false`、`0`、`""`。尤其不要套用
其他语言中“0 或空字符串是假”的经验。随后运行单元测试：

```powershell
cargo test --manifest-path book/code/v01-value/Cargo.toml `
  lua_truthiness_is_not_rust_truthiness -- --nocapture
```

如果预测错了，先修正 Lua 语义模型，再读 Rust 实现。

## 从“读到字节”到“识别一个值”

v0.0 只能读入脚本。本版本的唯一核心增量是 Lua 值模型与单字面量求值：`nil`、boolean、number 和 byte string。输入仍然不是一个完整 Lua 程序，只能包含一个字面量。

Lua 的关键语义从这里就开始锁定：

- 只有 `nil` 和 `false` 是假，`0` 和空字符串都是真；
- 不同类型不会因为展示文本相同就相等；
- Lua 5.1 字符串是字节序列，不是必须合法的 UTF-8 文本。

## Rust 学习点

`enum` 将“运行时才知道类型”变成编译器可检查的分支；`match` 强制每个语义函数面对全部 Value 变体。`Box<[u8]>` 是长度固定的拥有字节序列，比 `String` 更符合 Lua 合同。

```rust
{{#include ../../code/v01-value/src/lib.rs:value_model}}
```

`Option<f64>` 可以表达“这个分支是否能解析为数字”，而整个求值过程使用 `Result<Value, LiteralError>` 区分成功与带上下文的失败。

```rust
{{#include ../../code/v01-value/src/lib.rs:literal_eval}}
```

`Value` 是“同一时刻只处于一个变体”的和类型。匹配 `&Value` 时得到的是字段借用，适合
`type_name` 和 `is_truthy`；匹配拥有的 `Value` 时可以移动其中的字节，适合后续拼接。
这一差异不是语法细节，而是 API 是否需要消费输入的设计信号。

`ByteString(Box<[u8]>)` 同时作出两项承诺：内容是任意字节；构造结束后长度固定。
`Vec<u8>` 也拥有字节，但额外暴露 capacity 和增长能力；`String` 则额外承诺 UTF-8。
选择容器前，先写清楚 Lua 需要的合同，再选择最窄的 Rust 类型。

<a id="learning-code-map"></a>
## 代码阅读地图

1. `ByteString::from_bytes/as_bytes`：在拥有与借用视图之间切换；
2. `Value`：定义运行时可出现的四类值；
3. `Value::type_name/is_truthy/display`：三个穷尽 `match`；
4. `evaluate_literal`：分派字面量，返回 `Result<Value, LiteralError>`；
5. `parse_short_string`：把源码转义解码成语义字节；
6. `tests::arbitrary_string_bytes_are_preserved`：证明 NUL/high byte 没有丢失。

阅读时分别标出“源码切片的借用期”和“返回 Value 的拥有期”：函数结束后源码借用消失，
解码后的 `ByteString` 仍可独立存在，因此返回值不能继续引用临时解析缓冲区。

## 设计取舍

此时可以把字符串保存为 `String`，等遇到无效 UTF-8 再重构。那会让 Lexer、长度运算、标准库和 table key 都在后续反复修改。所以本书从第一个字符串开始就坚持字节语义；只有展示时才使用 `escape_default` 转成可读文本。

当前 `Value::String` 直接拥有字节。这会产生克隆成本，但对 v0.1 是最清晰的模型；等引入 table、函数与 GC 时再迁移到共享对象句柄。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v01-value/Cargo.toml -- `
  book/code/v01-value/examples/demo.lua
```

示例字符串含有 `\000` 与 `\255`，预期输出使用转义显示而不是损坏字节：

```text
type=string value="Lua bytes: \x00\xff"
```

```powershell
cargo test --manifest-path book/code/v01-value/Cargo.toml
```

验收要求覆盖 `nil/false/0/""` 真值、十六进制数、NUL、`0xff` 以及对非单字面量输入的非零退出码。

## 本版本暂不支持

- Token、运算符、名字与表达式；
- 字符串驻留、Table 与 GC 句柄；
- 隐式数值转换和元方法。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>证明 <code>"\000"</code> 的 Lua 字节长度是 1，并说明 C 字符串长度规则为什么不适用。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为每个 Value 变体增加 <code>type_name</code> 测试，并确认调用它不克隆或分配字符串。
  成功条件是现有测试与新增测试全部通过。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>在临时分支中把 <code>ByteString</code> 改成 <code>String</code>，运行
  <code>arbitrary_string_bytes_are_preserved</code>，记录最先无法表达或失败的边界，再撤销实验。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 不看源码写出 `Value` 的四个变体并解释各字段的 owner；
- 能区分 `Box<[u8]>`、`Vec<u8>` 和 `String` 作出的不同承诺；
- 能解释为什么 `is_truthy(&self)` 借用 Value，而解析函数返回拥有的 Value。

## 从本版迁往 v0.2

从 v0.0 迁移时，新增值模型和字面量错误，CLI 的成功输出从字节数改为类型与值。迁往 v0.2 时会删除“整份输入就是一个字面量”的假设，将字面量解码迁入 Lexer。更深入的成熟实现见 [Value](../ch02-runtime/value.md) 与 [ByteString](../ch02-runtime/byte-string.md) 专题。
