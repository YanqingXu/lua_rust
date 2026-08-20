# 03 · Table、所有权与共享对象

<div class="lesson-meta"><span>预计 25 分钟</span><span>Rust：Vec / HashMap / &mut</span><span>运行时核心</span></div>

## 一种结构，承担多种角色

Lua 用 table 同时表示数组、字典、对象、模块与环境：

```lua
local user = { "Rust", "Lua", name = "Ada" }
print(user[1])      -- Rust
print(user.name)    -- Ada
```

整数键适合连续存储，任意键适合哈希存储。项目选择混合布局：

```rust
{{#include ../../../crates/lua_core/src/table.rs:lua_table_layout}}
```

`array` 让小而连续的正整数键得到 O(1) 访问和良好缓存局部性；`hash` 负责字符串、对象和稀疏整数键。`hash_order` 与 `hash_positions` 额外维护稳定遍历位置，以处理 `next` 遍历期间删除当前键的情况。

## get 与 set 把 Lua 规则收进一个边界

```rust
{{#include ../../../crates/lua_core/src/table.rs:lua_table_get_set}}
```

读操作接收 `&self`，不会改变表；写操作必须接收 `&mut self`，Rust 保证同一时刻没有其他活跃借用偷偷观察到半完成状态。

注意几条 Lua 语义已经落成显式分支：

- 不存在的键读取为 `nil`；
- `nil` 和 NaN 不能作为键；
- 写入 `nil` 等同于删除；
- 可表示为小正整数的数字优先进入数组部分。

## 为什么返回克隆的 Value

`get` 返回 `Value`，而不是 `&Value`。调用方常常要在继续执行 VM 时修改 table 或栈；若长期持有指向 table 内部的引用，借用范围会扩散并阻止后续可变操作。

这里的 `clone` 通常复制立即值或 GC 句柄，不会深拷贝整张表。值表示的设计让 API 可以在清晰的借用边界与可接受的复制成本之间取平衡。

## Table 自身为什么需要 GC

Lua table 可以形成环：

```lua
local t = {}
t.self = t
```

单纯引用计数无法回收这种结构。项目让 `Value::Table` 保存 GC 句柄，并在 table 内继续保存其他 `Value`。这形成对象图，GC 通过根集合追踪可达对象。

目前仓库的 GC 闭环仍在演进。教学上最重要的不是假装问题已经消失，而是看见 Rust 静态所有权与动态对象图之间的边界：

```text
Rust 所有权：保证句柄和容器操作的局部安全
运行时 GC：决定动态对象图中的长期可达性
```

## 另一种方案：全部放 HashMap？

可以，代码会更短；代价是常见数组访问需要 hash，构造大列表的内存布局也更差。混合布局增加了迁移与删除逻辑，却更贴近 Lua table 的典型使用方式。这是运行时工程常见取舍：为了热点路径，接受受测试约束的内部复杂度。

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>运行 <code>cargo test -p lua_core table</code>，找出数组键迁移到 hash 或扩容的测试。</li>
    <li>画出 <code>local t={}; t.self=t</code> 对应的 Value 与 Table 引用图。</li>
    <li>思考：如果 <code>get</code> 返回 <code>&Value</code>，调用方紧接着执行 <code>set</code> 会遇到什么借用问题？</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>Table 的数组/哈希双存储是性能选择；&self 与 &mut self 表达访问权限；GC 处理 Rust 编译器无法静态推断的动态环状对象图。</p>
</div>

继续探索：[`table.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_core/src/table.rs) · [`runtime ownership RFC`](https://github.com/YanqingXu/lua_rust/blob/main/docs/rust_migration/runtime_ownership_rfc.md)
