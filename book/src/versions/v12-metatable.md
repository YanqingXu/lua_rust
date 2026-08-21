# v0.12 · Metatable 与元方法

<div class="lesson-meta"><span>核心增量：动态语义分发</span><span>workspace</span><span>checkpoint v0.12</span></div>

v0.11 已经能创建 closure、共享 upvalue 并执行完整的调用协议，但每种操作仍只认识 VM 内建类型。本版只增加一个核心能力：**值可以通过 metatable 为既有操作提供后备行为**。这把“对象系统”从语法特性变成了普通 table、函数和一套统一的事件协议。

## 本版完成后能做什么

```lua
local methods = {}
local mt = { __index = methods }

mt.__add = function(left, right)
    return setmetatable({ value = left.value + right.value }, mt)
end
mt.__call = function(self, scale)
    return self.value * scale
end
methods.double = function(self)
    return self.value * 2
end

local answer = setmetatable({ value = 20 }, mt)
             + setmetatable({ value = 22 }, mt)

return answer:double() + answer(10) -- 504
```

本版实现的事件如下：

| 操作 | 快速路径 | 后备事件 |
|---|---|---|
| `object[key]` | table 原始命中 | `__index` |
| `object[key] = value` | table 已有非 nil 键 | `__newindex` |
| `+ - * / % ^` | 两个 number | `__add` 到 `__pow` |
| 一元 `-` | number | `__unm` |
| `< <= > >=` | number/string 可直接比较 | 共享的 `__lt` / `__le` |
| `== ~=` | 原始相等 | 两个 table 共享的 `__eq` |
| `..` | string/number 可直接拼接 | `__concat` |
| `#` | string 或 table 原始长度 | `__len` 教学扩展 |
| `value(...)` | Lua/native function | `__call` |
| `tostring(value)` | 内建字节表示 | `__tostring` |

`setmetatable`、`getmetatable`、`rawget`、`rawset` 和 `tostring` 也进入全局环境。`__metatable` 非 nil 时，公开读取会得到保护值，再次修改则产生 `Metamethod` 错误。

## Raw 快速路径与元方法慢路径

每个 table 新增一个可选 `TableRef`。`get`、`set` 仍然只做原始访问，不在核心容器里偷偷触发 Lua 代码；VM 的操作分发才负责查找元方法。

```rust
{{#include ../../code/v12-metatable/crates/lua_core/src/lib.rs:table_storage}}
```

这个边界很重要：若 `TableRef::get` 本身递归执行 `__index`，实现 `rawget` 会变得困难，借用冲突也会和语言调用栈混在一起。本版采用两阶段模型：

1. 尝试 VM 能直接完成的 raw 操作；
2. 快速路径不适用时，从值的 metatable 原始读取事件字段；
3. 事件是函数就按普通调用协议执行，是 table 就继续索引/写入链；
4. 没有事件时，table 读取返回 nil，其他非法操作返回 `Type` 错误。

```rust
{{#include ../../code/v12-metatable/crates/lua_vm/src/lib.rs:metamethod_dispatch}}
```

`__index`、`__newindex` 和 `__call` 都可能把请求转交给另一个带 metatable 的值。为了让错误可预测，三类链统一限制为 64 层，循环或超长代理链返回 `VmErrorKind::Limit`，而不是耗尽 Rust 栈。

## 二元运算不是简单地“找一个函数”

算术和拼接先查左操作数，再回退到右操作数；调用元方法时仍按 `(left, right)` 传参。比较更严格：两个操作数必须提供身份相同的 handler。`a <= b` 没有共享 `__le` 时，按 Lua 规则回退为 `not (b < a)`；`>` 和 `>=` 则交换操作数复用同一套逻辑。

相等先执行原始比较。不同 table 只有共享同一个 `__eq` 函数时才调用它；两个 metatable 中恰好各写了源码相同但身份不同的 closure，不算共享 handler。这一规则避免比较行为只由其中一侧任意决定。

## 可调用值与多返回协议复用

调用指令不再立即拒绝非函数，而是沿 `__call` 解析：

```text
object(arg) → object.__call(object, arg)
```

若 `__call` 本身仍不是函数，就把上一层对象插到参数头部并继续解析。解析完成后仍进入 v0.10/v0.11 的 Lua/native 调用与多返回协议；tail call 也在解析后复用当前调用帧。因此元方法没有第二套隐藏执行器。

## Primitive metatable

Table 的 metatable 属于单个对象；nil、boolean、number、byte string 和 function 没有可写字段，本版由 VM 按类型名保存共享 metatable。例如：

```lua
settypemetatable("number", {
    __index = {
        double = function(self) return self * 2 end
    }
})

return (21):double() -- 42
```

`settypemetatable` 是为了让读者直接观察 primitive 分发而提供的**临时教学 API**，不是 Lua 5.1 base library 函数。v0.17 加入 debug 库和宿主边界时应迁移到相应能力，v1.0 不能把这个名字宣称为官方兼容接口。

## 一致的错误分类

`VmError` 新增可机器判断的 `VmErrorKind`：

- `Bytecode`：损坏的指令、常量或寄存器窗口；
- `Type`：值不支持当前操作，也没有合适的元方法；
- `Metamethod`：受保护 metatable、非法元方法返回值或分发内部错误；
- `Limit`：指令、调用深度或元方法链超限；
- `Runtime`：其余运行时失败。

行号、PC、消息和 traceback 仍然保留。测试不再依赖一整段展示文本来分辨循环、类型错误和元方法协议错误。

## Rust 知识映射与设计选择

- `Option<TableRef>` 表达每个 table 可有可无的 metatable；
- `HashMap<&'static str, TableRef>` 保存 VM 级 primitive metatable；
- 小型 `VmErrorKind` enum 把稳定分类和面向人的错误消息分离；
- raw 容器 API 与 `&mut Vm` 分发 API 分层，避免在 `RefCell` 借用期间回调 Lua；
- `Value` 的对象身份比较让共享 comparison handler 可以精确判断；
- 迭代链与显式上限代替无界递归，便于控制资源和测试失败路径。

一种替代设计是给每个 opcode 写一套专用元方法查找；这样容易让左右操作数顺序、错误分类和链限制逐渐分叉。本版集中到 `metamethod`、`first_metamethod`、`shared_metamethod` 和 `call_event`。另一种方案是在 Rust trait 上为每个 Value 实现运算，但 Lua 的行为由运行时 table 决定，不适合静态 trait 分发。

## 从 v0.11 迁移了什么

- 保留：Proto、closure/upvalue、寄存器调用帧和多返回协议；
- 重构：`GETTABLE`、`SETTABLE`、算术、比较、拼接、长度和 CALL 从封闭类型判断改为“raw 快速路径 + 元方法慢路径”；
- 新增：Table 的可选 metatable 与 VM 的 primitive metatable 表；
- 新增：`setmetatable`、`getmetatable`、`rawget`、`rawset`、`tostring` 和临时 `settypemetatable`；
- 新增：`__index`、`__newindex`、算术、比较、拼接、长度、调用和字符串转换事件；
- 新增：64 层链保护与 `VmErrorKind`；
- 没有提前加入：GC heap、userdata、coroutine 或标准库适配器。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v12-metatable/Cargo.toml -- `
  --dump-bytecode book/code/v12-metatable/examples/demo.lua
cargo test --manifest-path book/code/v12-metatable/Cargo.toml --workspace
```

示例把 vector 的方法、加法、长度、调用和字符串转换组合起来，最终输出应为：

```text
type=number value=8348
```

本快照共有 65 个测试：49 个端到端测试、7 个编译器测试、7 个 core 测试和 2 个 VM 验证测试。新增覆盖 table/function 两类索引代理、写入转发与 raw 绕过、左右操作数回退、全部算术族、比较共享规则、`<=` 回退、primitive/function metatable、保护标记、三类链循环和 `__tostring` 返回类型。

## 明确的兼容性记录

本版为了在一个 checkpoint 中教学完整动态分发，让 table 的 `#value` 优先尝试 `__len`。**这是教学扩展，不是 Lua 5.1.5 行为**：官方 5.1.5 对 table 使用原始长度，不调用 table 的 `__len`。到 v1.0 必须选择移除该扩展，或把它写入 deviation registry；不得静默近似。

`settypemetatable` 同样是教学期宿主入口，最终公开接口需迁移。除这两项已登记差异外，本版仍只是阶段性子集，不能单独宣称 Lua 5.1 兼容。

## 本版本暂不支持

- 没有 GC，table/metatable、closure 和 upvalue 形成的 `Rc` 环无法回收；
- 没有 weak key/value、finalizer、userdata 和 userdata 元方法；
- 没有 coroutine，元方法不能跨 yield 暂停；
- 没有完整 base/math/string/table/io/os/package/debug 库；
- 没有 Lua 文件模块、REPL、binary chunk、C API 或动态 native module；
- table `#` 的边界仍沿用教学数组长度，不代表 Lua 5.1 未定义边界的全部行为。

## 章末练习

1. 用 `__index` 和 `__newindex` 实现一个把读写都记录到日志 table 的 proxy，并用 `rawget` 证明日志不会递归触发自身。
2. 为二维向量实现 `__sub`、`__eq` 与 `__tostring`，再故意让左右 metatable 使用两个不同的 `__eq` closure，观察共享 handler 规则。
3. 把元方法链上限从 64 改为 3，构造四层 `__index` 代理并检查 `VmErrorKind::Limit`。
4. 删除 CALL 前的 `resolve_callable`，解释普通调用和 tail call 哪些测试会分别失败。
5. 将 primitive metatable 存储从字符串 key 改为 `ValueKind` enum，比较两种表示在可扩展性和非法状态上的差异。

## 与成熟实现和专题章节的对应

- 当前成熟实现中 table header、类型 metatable 和统一事件分发，是本版模型迁移到 GC heap 后的目标；
- [Table、所有权与共享对象](../ch02-runtime/table.md) 可对照为什么本阶段仍使用 `Rc<RefCell<_>>`；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可观察同一 opcode 如何获得动态慢路径；
- 下一 checkpoint：[v0.13 · Runtime 所有权与完整 GC](v13-runtime-gc.md)，用 Runtime 唯一拥有 heap，并以 `GcRef` 取代对象图中的 `Rc`。
