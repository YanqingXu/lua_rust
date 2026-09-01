# v0.8：Table

<div class="lesson-meta"><span>Checkpoint 9 / 20</span><span>共享可变性</span><span>对象身份</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>第一次面对共享可变对象</strong>
  <p>前置：v0.7 的 Value、寄存器与赋值顺序。新知识：Vec/HashMap 混合存储、Eq/Hash、
  Rc 对象身份、RefCell 运行时借用。完成后你应能解释别名更新和引用环为何同时出现。</p>
</div>

<a id="learning-predict"></a>
## 先预测再运行

先判断：`local a={}; local b=a; b.x=1; return a.x` 返回多少？再判断两个分别创建、内容
相同的 `{x=1}` 是否相等。前者返回 1，因为 clone 句柄不复制对象；后者不相等，因为
Table 比较身份。最后画出 `t.self=t` 的强引用环，并预测 drop 后 strong count。

## 本版本增量

v0.7 的 Value 只包含立即值与字节字符串，无法表示共享对象图。v0.8 的唯一核心增量是 Lua Table：混合构造器、`t[k]`、`t.name`、索引赋值、对象身份和共享更新。

```lua
local inventory = {3, 5, name = "bolts", [10] = 99}
local alias = inventory
alias[2] = 7
inventory.self = inventory
```

<a id="learning-code-map"></a>
## 代码阅读地图

1. `TableKey`：只允许可哈希、符合 Lua 合同的 key；
2. `Table`：array part 与 map part 的 owner；
3. `Table::set/get`：1-based 转换、nil 删除与连续 tail 晋升；
4. `TableRef(Rc<RefCell<Table>>)`：共享身份与运行时借用边界；
5. constructor/index/LValue codegen：先保存地址，再求完右值；
6. VM table opcode：从寄存器 clone 句柄后再尝试可变借用；
7. alias、NaN key、自引用测试：分别锁定共享、错误和泄漏边界。

把 `Rc` 和 `RefCell` 分开理解：`Rc` 回答“谁共同拥有对象”，`RefCell` 回答“共享 owner
怎样临时获得可变访问”。`try_borrow_mut` 的失败是运行时错误，不是线程同步；本版仍是
单线程，而且引用环无法靠 RefCell 解决。

## Table 既不是 Vec 也不只是 HashMap

连续正整数 key 使用 `Vec<Value>` 保存，其他 key 使用 `HashMap<TableKey, Value>`。Lua 下标从 1 开始，所以 key=1 对应 `array[0]`。稀疏整数先进入 map；当中间空洞被填补时，连续 tail 会被晋升回 array。

写入 `nil` 表示删除 key。如果删除数组尾部，实现会继续删掉尾部 nil，使教学版 `#t` 返回连续 array part 长度。对带空洞 Table 的 `#` 在 Lua 中本就存在边界不确定性，不应用它计算稀疏 key 数量。

先把容器规则压缩成两条路径：

```text
set(positive contiguous integer, value) → array[index - 1]
set(other valid key, value)             → map[TableKey]
set(any key, nil)                       → delete, trim array tail
fill next array hole                    → promote contiguous integer keys from map
```

`TableRef` 只在这组容器规则外增加身份和运行时借用。
[查看完整 Table/TableRef 实现](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v08-table/crates/lua_core/src/lib.rs)

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

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>创建两个内容相同的 Table，解释身份相等与深度相等的差别，并说明为什么函数也会
  选择身份相等。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>写入 key=3、1、2，增加测试锁定 key=3 从 map 晋升到 array 的时机；再断言
  <code>-0.0</code> 与 <code>0.0</code> 命中同一 slot。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>创建并删除自引用 field，比较 strong count；说明手工断环为什么只能修这个例子，
  不能作为任意 Lua 对象图的回收算法。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出两个 Value 别名、Rc control block 与 RefCell/Table 的关系；
- 能说明一次 set 如何在 array/map 间选择并保持 1-based 合同；
- 能列出 nil、NaN、-0.0 key 的不同处理；
- 能解释本版模型为什么安全但会泄漏环。

## 从本版迁往 v0.9

从 v0.7 迁移时新增 `Table/TableRef/TableKey`、`Value::Table`、constructor/index AST、复合 LValue 与四条 Table opcode。v0.9 将在同一共享对象模型上加入 Function、嵌套 Proto 和调用帧。成熟实现对应 [Table 专题](../ch02-runtime/table.md)。
