# v0.13 · Runtime 所有权与完整 GC

<div class="lesson-meta"><span>核心增量：可回收的对象图</span><span>workspace</span><span>checkpoint v0.13</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>从引用计数迁移到唯一 Heap owner</strong>
  <p>前置：v0.12 的 Rc 对象图、closure/upvalue 与 metatable。新知识：typed handle、
  PhantomData、arena、roots/edges、迭代 mark-sweep 与 stale/foreign 校验。完成后你应能
  为一张对象图手工执行完整 GC。</p>
</div>

v0.12 的 table、closure 和 upvalue 都由 `Rc` 强引用。这个模型便于先讲清共享身份，却无法回收 `table.self = table`，closure 捕获自身也会形成环。本版只完成一个核心迁移：**Runtime 成为所有 heap 对象的唯一 owner，语言值只携带经过校验的非拥有 handle，并加入一次完整 mark/sweep 回收**。

<a id="learning-predict"></a>
## 先预测再运行

画两张图：一张自引用 table 已离开 scope；另一张同样的 table 仍被 global 指向。分别从
roots 开始涂色，前者必须在 sweep 删除，后者必须保留。再把 global 改成宿主持有的裸
GcRef：非拥有 handle 不是 root，若不登记 registry，对象仍可被回收并使 handle stale。

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

<a id="learning-code-map"></a>
## 代码阅读地图

1. `GcRef<T>`：heap_id/object_id 加类型标记，不拥有对象；
2. `HeapRecord`：唯一拥有 Table/Closure/Upvalue 等 payload；
3. handle lookup：依次验证 heap、object existence 和 kind；
4. VM root collector：active frames、globals、registry、metatables、temporaries；
5. object tracer：从 Table/Closure/Upvalue 枚举 edges；
6. mark stack：只访问未标记对象并继续压入 children；
7. sweep 与失败恢复：删除 white，或在错误时清空本轮 mark；
8. shutdown：唯一 owner 一次性 drain 全部 record。

本章第一次要求把 Rust owner 与语言可达性彻底分开：Heap 是 Rust owner；GcRef 只是逻辑
身份；GC roots 决定 Lua 对象是否继续存在。引用计数把这三件事混在 strong count 中，
因此无法表达任意环。

## GcRef 不拥有对象

`GcRef<T>` 只保存两个整数和一个零大小类型标记：

```text
GcRef<TableObject> { heap_id, object_id }
             │          │
             │          └─ 在一个 heap 内单调递增且不复用
             └─ 防止把另一个 Runtime 的 handle 混入当前对象图
```

访问句柄时始终遵守同一验证顺序：

```text
handle.heap_id != heap.id       → ForeignHandle
records lacks handle.object_id  → StaleHandle
record kind != GcRef<T> kind    → WrongKind
otherwise                       → borrow payload through Heap
```

[查看完整 GcRef/Heap 实现](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v13-runtime-gc/crates/lua_core/src/lib.rs)

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

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>为允许 slot 复用的 <code>(object_id,generation)</code> handle 列出防止 ABA 所需的验证，
  并与本版“ID 永不复用”比较。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>构造两个 table 的环，分别在 global 保留和删除后比较 GcStats；再设计 HostRoot RAII
  guard，让构造/Drop 对称更新 registry。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>暂时删除 primitive metatable root 或 CLOSE 后的寄存器清理，运行定向测试；分别解释
  一个造成误回收、另一个造成假存活的原因。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能区分 owner、handle、root 和 edge；
- 能列出完整 root inventory 并手推 mark/sweep；
- 能说明 foreign、stale、wrong-kind 三类失败；
- 能解释为什么 close upvalue 必须先于清空死亡寄存器。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 Runtime/Heap、root visitor 和对象 header 是本版 arena 的性能化映射目标；
- [Table、所有权与共享对象](../ch02-runtime/table.md) 可回顾 v0.8 的 `Rc<RefCell<_>>` 为什么只是阶段模型；
- [用 enum 表示 Lua Value](../ch02-runtime/value.md) 可对照拥有数据的 primitive 与非拥有 `GcRef`；
- 下一 checkpoint：[v0.14 · 增量 GC、弱表与 Finalizer](v14-incremental-gc.md)，把完整 mark/sweep 拆成可暂停阶段。
