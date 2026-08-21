# v0.13 · Runtime 所有权与完整 GC

<div class="lesson-meta"><span>核心增量：可回收的对象图</span><span>workspace</span><span>checkpoint v0.13</span></div>

v0.12 的 table、closure 和 upvalue 都由 `Rc` 强引用。这个模型便于先讲清共享身份，却无法回收 `table.self = table`，closure 捕获自身也会形成环。本版只完成一个核心迁移：**Runtime 成为所有 heap 对象的唯一 owner，语言值只携带经过校验的非拥有 handle，并加入一次完整 mark/sweep 回收**。

## 本版完成后能做什么

```lua
do
    local cycle = {}
    cycle.self = cycle
end

local reclaimed = gc_collect()
return reclaimed >= 1 -- true
```

同一机制也必须证明相反方向：只要从 stack、global、registry、closure/upvalue 或 metatable 可达，对象就不能被误回收。

## GcRef 不拥有对象

`GcRef<T>` 只保存两个整数和一个零大小类型标记：

```text
GcRef<TableObject> { heap_id, object_id }
             │          │
             │          └─ 在一个 heap 内单调递增且不复用
             └─ 防止把另一个 Runtime 的 handle 混入当前对象图
```

```rust
{{#include ../../code/v13-runtime-gc/crates/lua_core/src/lib.rs:table_storage}}
```

复制 `GcRef` 不增加引用计数，也不延长对象寿命。Heap arena 的 `HashMap<object_id, HeapRecord>` 才拥有 Table、Closure 和 Upvalue。每次访问先检查 heap ID、对象是否仍存在以及对象种类：foreign handle 返回 `ForeignHandle`，已经 sweep 的 ID 返回 `StaleHandle`，不会解引用悬空地址。

Object ID 永不复用，因此旧 handle 不会在下一次分配后意外指向另一个对象。展示身份时组合 heap ID 与 object ID；Lua 的 table/function 相等仍比较 handle 身份。

## 为什么先用安全 arena

生产 GC 常用指针压缩、页分配器和受控 `unsafe` 来减少间接访问成本。本 checkpoint 先把危险边界压缩到零：arena 查表、typed handle 和显式错误足以教学对象图不变量，全快照没有 `unsafe`。读者仍应掌握受控 `unsafe` 的审核标准：唯一 owner、指针稳定期、写屏障覆盖、sweep 后失效与任何回调重入点都必须能逐项证明。

替代方案是 `Rc<RefCell<_>>` 加 `Weak` 手工拆环，但 Lua 程序能动态构造任意强连通图，无法要求用户选择哪条边为 weak。另一个方案是直接把裸指针放进 Value；它更快，却会把 foreign/stale 检查和对象移动约束扩散到整个 VM，不适合这一教学阶段。

## Mark：根集合必须完整

VM 在 stop-the-world 回收点收集以下 roots：

| 根 | 为什么需要 |
|---|---|
| 活跃 frame 的寄存器 | local、参数与尚未消费的结果 |
| global table 映射 | 脚本全局可达值 |
| registry | 宿主显式固定的值 |
| 活跃 Function 与 Proto | 当前执行模板、常量和嵌套 Proto |
| open upvalue map | closure 正在共享的栈位置 |
| primitive metatable | 类型级动态行为 |
| temporary roots | 元方法/调用参数在写回寄存器前的短暂存活期 |

```rust
{{#include ../../code/v13-runtime-gc/crates/lua_vm/src/lib.rs:gc_roots}}
```

Closure 会标记它的 Proto 常量和全部 upvalue；upvalue 再标记 open stack slot 或 closed Value；Table 会遍历数组、哈希 key/value 和 metatable。因此 root walker 最终覆盖完整对象图，而不是只扫一层 table。

宿主从 `execute` 取回 table/function handle 后，若要跨显式 GC checkpoint 持有它，应先写入 registry；清掉 registry entry 后对象才重新具备可回收性。测试同时覆盖固定和解除固定。

## Scope 退出也是 GC 正确性的一部分

编译器从 v0.11 起会在块、break 和每轮 for 退出时发出 `CLOSE A`。此前它只关闭 open upvalue，寄存器里仍可能残留已经离开词法作用域的 table。若 GC 扫描这些残值，自环永远看起来可达。

v0.13 的 `CLOSE A` 先把 upvalue 从位置迁移成 closed Value，再把 `R[A..]` 清为 nil：

```text
活跃 local ── CLOSE ──> closed upvalue（若被捕获）
     │
     └───────────────> stack slot = nil
```

这样逃逸 closure 仍保留值，普通死亡 local 则不再成为假 root。调用帧结束时也会统一关闭 upvalue，并从 active-frame root 集合移除。

## Sweep 与失败恢复

完整回收先清点对象数量，再从 roots 做迭代 mark，最后保留 marked record 并重置 mark bit：

```rust
{{#include ../../code/v13-runtime-gc/crates/lua_core/src/lib.rs:mark_sweep}}
```

遍历遇到 foreign/stale handle 或 stack borrow 冲突时，本次回收失败，已经设置的 mark bit 会全部复位；不会在半标记状态继续 sweep。成功结果返回 `GcStats { before, collected, remaining }`，便于测试和后续 allocator accounting。

本版使用 stop-the-world 完整回收，所以 mutator 与 marker 不会并行修改对象图，尚不需要 write barrier。v0.14 将把同一不变量拆成可暂停阶段。

## Runtime 关闭

`Heap::shutdown` 一次清空所有 record、返回释放数量并拒绝后续分配/访问。`Vm::drop` 始终调用它，因此正常结果、运行时错误和宿主提前结束都走同一 drain 路径。对象资源现在由唯一 owner 统一释放，不再依赖能否打破 `Rc` 环。

栈槽仍用 `Rc<RefCell<Value>>` 提供 open upvalue 所需的稳定位置，但它不拥有任何 HeapObject；slot 中的 `GcRef` 仍是非拥有 handle。Table、Closure、Upvalue 三类 heap 身份已经全部迁出 `Rc`。

## Rust 知识映射与设计选择

- `PhantomData<fn() -> T>` 给整数 handle 增加编译期对象类型；
- `AtomicU64` 为每个 Heap 分配进程内唯一 ID；
- 单调 object ID 避免 ABA 式 stale-handle 复活；
- `HashMap<u64, HeapRecord>` 表达唯一所有权和稳定逻辑身份；
- `Result<_, HeapError>` 让 foreign、stale、wrong-kind、shutdown 可测试；
- 显式 graph traversal 训练 roots、edges 与三色算法之前的基本不变量；
- 受控 `unsafe` 作为性能替代路线被审计，而不是提前散布进 Value API。

## 从 v0.12 迁移了什么

- 删除：Table/Closure/Upvalue 对 HeapObject 的 `Rc` 强所有权；
- 新增：typed `GcRef<T>`、唯一 Heap owner、heap/object ID 与句柄校验；
- 重构：所有 table、function、upvalue 访问都显式经过 Heap；
- 新增：完整 mark/sweep、root registry、active frame/function 与 temporary root 跟踪；
- 重构：`CLOSE` 在关闭 upvalue 后清理死亡寄存器；
- 新增：确定性 `shutdown` 和失败时 mark bit 恢复；
- 新增：临时教学诊断 `gc_collect`、`gc_live`；
- 保留：v0.12 的 metatable 语义与已登记的 table `__len` 教学扩展。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v13-runtime-gc/Cargo.toml -- `
  book/code/v13-runtime-gc/examples/demo.lua
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --workspace
```

示例先回收一个不可达自环，再证明 closure 捕获的另一个自环仍存活：

```text
type=number value=142
```

本快照共有 72 个测试：52 个端到端测试、7 个编译器测试、10 个 core 测试和 3 个 VM 测试。新增验收覆盖自环回收、可达图保留、调用者 frame、global、metatable、closure/upvalue、registry、foreign/stale handle、shutdown 和回收统计。

## 本版本暂不支持

- 只有显式 stop-the-world 完整回收，没有自动 allocator checkpoint；
- 没有增量 phase、gray queue 持久化或 write barrier；
- 没有 weak key/value、ephemeron、finalizer、resurrection 和二次死亡；
- 没有 userdata 与 coroutine；
- `gc_collect` / `gc_live` 是教学接口，不是最终 `collectgarbage` 合同；
- arena 以正确性和可读性优先，尚未实现页分配、压缩或性能基线。

## 章末练习

1. 构造两个 table 相互引用的环，分别在 global 保留和删除 global 后运行完整 GC，比较 `GcStats`。
2. 暂时删除 primitive metatable roots，写一个在回收后访问 number 方法的失败测试。
3. 不清理 `CLOSE A` 后的寄存器，观察自环测试为何不再回收，再解释 upvalue 为什么必须先关闭。
4. 用 `(object_id, generation)` 且允许 slot 复用重写 handle，列出防止 ABA 所需的检查。
5. 设计一个 `HostRoot` RAII guard：构造时写 registry，Drop 时清 entry；说明它如何处理嵌套宿主调用。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 Runtime/Heap、root visitor 和对象 header 是本版 arena 的性能化映射目标；
- [Table、所有权与共享对象](../ch02-runtime/table.md) 可回顾 v0.8 的 `Rc<RefCell<_>>` 为什么只是阶段模型；
- [用 enum 表示 Lua Value](../ch02-runtime/value.md) 可对照拥有数据的 primitive 与非拥有 `GcRef`；
- 下一 checkpoint：[v0.14 · 增量 GC、弱表与 Finalizer](v14-incremental-gc.md)，把完整 mark/sweep 拆成可暂停阶段。
