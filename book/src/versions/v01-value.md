# v0.1：Lua Value 与字面量

<div class="lesson-meta"><span>Checkpoint 2 / 20</span><span>动态值</span><span>字节语义</span></div>

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

## 练习与迁移

1. 为 `Value` 增加一个不分配内存的 `type_name` 测试。
2. 证明 `"\000"` 的 Lua 长度应为 1，而不是 C 字符串的 0。
3. 尝试把 `ByteString` 换成 `String`，记录哪个测试第一个失败。

从 v0.0 迁移时，新增值模型和字面量错误，CLI 的成功输出从字节数改为类型与值。迁往 v0.2 时会删除“整份输入就是一个字面量”的假设，将字面量解码迁入 Lexer。更深入的成熟实现见 [Value](../ch02-runtime/value.md) 与 [ByteString](../ch02-runtime/byte-string.md) 专题。
