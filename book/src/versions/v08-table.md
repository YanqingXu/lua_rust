# v0.8：Table

<div class="lesson-meta"><span>Checkpoint 9 / 20</span><span>共享可变性</span><span>对象身份</span></div>

## 本版本增量

v0.7 的 Value 只包含立即值与字节字符串，无法表示共享对象图。v0.8 的唯一核心增量是 Lua Table：混合构造器、`t[k]`、`t.name`、索引赋值、对象身份和共享更新。

```lua
local inventory = {3, 5, name = "bolts", [10] = 99}
local alias = inventory
alias[2] = 7
inventory.self = inventory
```

## Table 既不是 Vec 也不只是 HashMap

连续正整数 key 使用 `Vec<Value>` 保存，其他 key 使用 `HashMap<TableKey, Value>`。Lua 下标从 1 开始，所以 key=1 对应 `array[0]`。稀疏整数先进入 map；当中间空洞被填补时，连续 tail 会被晋升回 array。

写入 `nil` 表示删除 key。如果删除数组尾部，实现会继续删掉尾部 nil，使教学版 `#t` 返回连续 array part 长度。对带空洞 Table 的 `#` 在 Lua 中本就存在边界不确定性，不应用它计算稀疏 key 数量。

```rust
{{#include ../../code/v08-table/crates/lua_core/src/lib.rs:table_storage}}
```

## key 的类型和不变量

`nil` 不能作为 key，NaN 也被拒绝；两者都返回 `TableError` 而不是 panic。`-0.0` 与 `0.0` 会先归一化，符合数值相等。boolean、number、byte string 和 Table 都可以作为 key。

Table key 必须保存 `TableRef` 本身，不能只保存裸指针整数。否则 key 对象可能被释放，其地址又被新对象复用，导致身份错乱。`TableRef` 的 `Eq/Hash` 使用 `Rc` 指针身份，但 key 仍持有强引用。

## Rc、RefCell 与别名

`Value::Table(TableRef)` 克隆只增加 `Rc` 计数，不克隆 Table 内容。因此两个 Value 句柄观察到同一更新，相等比较也按对象身份而不是深度内容比较。

`RefCell` 将 Rust 借用检查移到运行时。本快照使用 `try_borrow/try_borrow_mut`，重入借用冲突会返回 `TableError::BorrowConflict`，而不是触发 `RefCell` panic。VM 在借用 Table 前先从寄存器克隆 key/value，避免在可变借用期间又借用寄存器窗口。

## AST、左值与指令

Parser 将 `.name` 立即降级为字符串 key 的索引节点，所以 `t.name` 与 `t["name"]` 共用同一 Codegen/VM 路径。Table constructor 区分 list field 与 record field：

- `{10, 20}` 用 `SETLIST` 写入 1、2；
- `{name = "lua"}` 将 name 变成 byte-string key；
- `{[expression] = value}` 先求 key，再求 value，使用 `SETTABLE`。

Table 赋值目标不只是名字。Codegen 在求右值前先把所有 `table/key` 地址保存到临时寄存器，然后统一写入。这使 `t[i], i = 42, 2` 仍使用赋值前的 `i` 定位 Table slot。

VM 增加 `NEWTABLE`、`GETTABLE`、`SETTABLE` 和 `SETLIST`：

```rust
{{#include ../../code/v08-table/crates/lua_vm/src/lib.rs:table_vm}}
```

## 引用环：本版本的有意限制

```lua
local t = {}
t.self = t
```

这段脚本让 Table 内部的 Value 强引用 Table 自身。即使脚本外部不再持有 `t`，`Rc` strong count 仍不会归零，所以对象无法回收。测试明确观察 strong count 增加；本版本不用 `Weak` 偷偷改变 Lua 强引用语义。v0.13 将用运行时管理的 `GcRef` 和 mark-sweep 解决这个对象图问题。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v08-table/Cargo.toml -- `
  --dump-bytecode book/code/v08-table/examples/demo.lua
```

预期结果：

```text
type=number value=20
```

```powershell
cargo test --manifest-path book/code/v08-table/Cargo.toml --workspace
```

验收覆盖 1-based array、稀疏整数晋升、字符串/混合 field、nil 删除、nil/NaN key 错误、非 Table 索引错误、别名共享更新、身份相等、左值地址求值顺序和自引用。v0.0–v0.7 的全部回归也必须保持通过。

## 本版本暂不支持

- Metatable 与任何元方法；
- `pairs/ipairs/next` 与 generic for；
- 真实 GC、弱表和 finalizer；
- 函数值、userdata 和 thread key，对应 Value 类型将在后续版本增加。

## 练习与迁移

1. 依次写入 key=3、1、2，观察 key=3 什么时候从 map 晋升到 array。
2. 证明 `-0.0` 与 `0.0` 读写同一个 slot。
3. 创建两个内容相同的 Table，解释为什么它们不相等。
4. 删除自引用 field 并比较 strong count，再说明为什么通用 Lua GC 不能依赖用户手动断环。

从 v0.7 迁移时新增 `Table/TableRef/TableKey`、`Value::Table`、constructor/index AST、复合 LValue 与四条 Table opcode。v0.9 将在同一共享对象模型上加入 Function、嵌套 Proto 和调用帧。成熟实现对应 [Table 专题](../ch02-runtime/table.md)。
