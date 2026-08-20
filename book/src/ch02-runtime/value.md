# 01 · 用 enum 表示 Lua Value

<div class="lesson-meta"><span>预计 20 分钟</span><span>Rust：enum / match</span><span>运行时核心</span></div>

## 从一个“类型会变”的变量开始

Lua 允许同一个变量先后保存不同类型：

```lua
local value = 42
value = "forty-two"
value = { answer = 42 }
```

这并不意味着一个值在内存中凭空改变了类型。真正发生的是：每次赋值都产生一个带有**类型标签**和**载荷**的运行时值，变量只是改为引用或保存另一个值。

Rust 变量的类型在编译期固定，恰好可以用一个枚举统一描述“所有允许出现的 Lua 类型”。

## 真实实现

下面的代码直接来自 `lua_core/src/value.rs`：

```rust
{{#include ../../../crates/lua_core/src/value.rs:lua_value}}
```

`Value` 是一个 sum type：任意时刻它只会是其中一个 variant。`Number(f64)` 直接携带数字，`Boolean(bool)` 直接携带布尔值；字符串、表、函数等可回收对象则携带 `GcRef<T>`。

这给我们三个重要保证：

1. 不会出现“标签说是数字，载荷其实是表”的未定义组合；
2. 编译器可以检查模式匹配是否覆盖了所有 variant；
3. 每个 variant 的所有权策略能被写进类型，而不是散落在约定里。

## match 是运行时分发的第一层

获取 Lua 类型标签时，代码必须显式处理每种值：

```rust
{{#include ../../../crates/lua_core/src/value.rs:lua_value_type}}
```

如果以后增加一种 `Value`，这个 `match` 会提示我们补齐分支。对于解释器而言，这种“改动时主动报错”非常宝贵，因为同一种值会流经算术、比较、表索引、标准库和 GC。

## Lua 真值语义不是 Rust bool

Lua 5.1 中只有 `nil` 和 `false` 为假，数字 `0`、空字符串和空表都为真。因此不能写“把 Value 转成 Rust bool”这样的模糊接口，语义应该直接表达出来：

```rust
pub fn is_false(&self) -> bool {
    matches!(self, Value::Nil | Value::Boolean(false))
}

pub fn is_true(&self) -> bool {
    !self.is_false()
}
```

试着预测：

```lua
if 0 then print("zero is true") end
if "" then print("empty string is true") end
```

两行都会打印。这是语言语义，不是实现细节。

## 为什么有 `as_number` 和 `try_as_number`

同一种读取操作可能有两种调用场景：

- VM 已经通过 opcode 或前置检查确认类型，失败代表内部不变量被破坏；
- 标准库正在处理用户输入，类型不符是普通运行时错误。

前者可以使用直接访问器，后者更适合返回 `Option` 或进一步转换为 `Result`。Rust 的返回类型把“谁负责处理失败”写到了调用点。

## 设计取舍：为什么不使用 trait object

`Box<dyn LuaValue>` 看起来更面向对象，但会带来额外间接访问，也难以穷尽检查所有 Lua 类型。VM 的热点路径频繁按类型分发，紧凑枚举更直接。另一方面，大型对象不能全塞进 enum，否则复制 `Value` 会变重，因此项目只在 variant 中保存 GC 句柄。

这形成一个清晰边界：

```text
立即值：Number / Boolean / Nil  → 直接放进 Value
共享对象：String / Table / ...  → Value 保存 GcRef<T>
```

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>在 <code>crates/lua_core/tests/value_tests.rs</code> 中找到真值测试，补充 <code>0</code> 和空字符串场景。</li>
    <li>写一个函数 <code>fn describe(value: &Value) -&gt; &'static str</code>，使用穷尽 <code>match</code> 返回类型说明。</li>
    <li>思考：为什么 <code>describe</code> 接收 <code>&Value</code> 而不是取得 <code>Value</code> 的所有权？</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>你应该能解释：动态类型并不要求实现语言也是动态类型；Rust enum 将类型标签与载荷绑定，而 GC 句柄控制大型共享对象的表示。</p>
</div>

继续探索：[`value.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_core/src/value.rs) · [`types.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_core/src/types.rs)
