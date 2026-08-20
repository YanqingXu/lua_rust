# 02 · Lua 字符串不等于 Rust String

<div class="lesson-meta"><span>预计 20 分钟</span><span>Rust：切片 / UTF-8 / 所有权</span><span>运行时核心</span></div>

## 先拆掉一个危险假设

Rust 的 `String` 保证内容是 UTF-8，Lua 5.1 字符串却是任意字节序列。它可以包含：

- `0x00`，也就是嵌入的 NUL；
- `0xff` 等无效 UTF-8 字节；
- 看起来像文本、实际上用于二进制协议的内容。

如果运行时直接使用 `String`，不是无法表示这些值，就是会在某个边界悄悄替换或拒绝字节。这属于语义错误。

## 用字节作为规范表示

项目使用 `ByteString` 保存不可变字节，并额外维护尾部 NUL：

```rust
{{#include ../../../crates/lua_core/src/byte_string.rs:lua_byte_string}}
```

这里有两个不同的长度：

```text
逻辑值 b"a\0b"        长度 3
底层存储 [a, 0, b, 0]  长度 4（最后一个 0 是哨兵）
```

`as_bytes()` 永远排除哨兵，而 `as_ptr()` 必须与显式 `len()` 一起使用。嵌入 NUL 是数据，不能把指针当成普通 C 字符串后依赖首个 `\0` 结束。

## 借用让“查看”不产生复制

`as_bytes(&self) -> &[u8]` 返回借用切片。切片只描述起点与长度，数据仍由 `ByteString` 拥有。返回值的生命周期不能超过 `self`，Rust 编译器替我们守住了这个约束。

这与 `from_bytes(bytes: &[u8]) -> Self` 正好相反：构造函数复制输入，返回的新对象拥有自己的存储，因此不再受输入切片生命周期限制。

## 文本转换必须显式

运行时内部以字节为真相，只有在人类可见的边界才尝试解释为文本：

- `to_utf8()`：要求有效 UTF-8，失败返回 `Utf8Error`；
- `to_string_lossy()`：允许展示，但会用替换字符标记无效序列；
- 文件路径、错误消息和终端输出：各自明确选择策略。

“转换”这个动作没有被隐藏在 `Display` 或隐式 coercion 中，因此调用者必须意识到信息可能丢失。

## 不可变性与驻留

Lua 字符串不可变。不可变值适合计算一次 hash，并在字符串池中共享相同内容。项目进一步用 `GcString` 和 `StringPool` 管理运行时身份：`ByteString` 解决字节语义，字符串池解决复用，GC 句柄解决生命周期。不要把三个问题混成一个万能类型。

<div class="warning">
  <strong>边界提醒</strong>
  <p>尾部 NUL 是为了兼容指针 + 长度形式的边界，不代表任意 Lua 字符串都能安全传给期望 C 字符串的 API。</p>
</div>

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>运行 <code>cargo test -p lua_core byte_string</code>，阅读嵌入 NUL 与无效 UTF-8 测试。</li>
    <li>构造 <code>ByteString::from_bytes(&[0xff, 0, b'A'])</code>，比较 <code>to_utf8</code> 与 <code>to_string_lossy</code>。</li>
    <li>解释为什么 <code>as_bytes</code> 可以返回切片，而 <code>from_bytes</code> 选择复制输入。</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>Rust String 是 UTF-8 文本容器，Lua string 是字节值。正确建模从保留全部字节开始，文本解释只能发生在明确边界。</p>
</div>

继续探索：[`ByteString RFC`](https://github.com/YanqingXu/lua_rust/blob/main/docs/rust_migration/byte_string_rfc.md) · [`string_pool.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_core/src/string_pool.rs)
