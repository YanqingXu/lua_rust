# v0.14：从弱边分类到 ephemeron 固定点

本包接续短借用和 finalizer 事件问题链，单独研究两个问题：

1. 如果扫描时把所有边都当 strong，事后按 `__mode` 删除条目还来得及吗？
2. 弱 key 已活才传播 value；传播一轮，跨另一张弱表的链就完整了吗？

只修改 [starter.rs](starter.rs) 的两个 TODO。每题的共享合同使用另一个健康 hook，
因此前题的错误不会掩盖后题。固定 [faults.rs](faults.rs) 和 CLI 不调用 starter；
默认通过只能证明反例与参考设计可重放，不能说明学习者已经完成 TODO。

## 先预测，再重放

从仓库根目录运行：

```powershell
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example weak_problem_chain -- weak
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example weak_problem_chain -- ephemeron
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example weak_problem_chain -- real
```

省略参数或使用 `all` 会重放全部。CLI 退出码 0 表示重放执行完成，行为正确性另由测试
断言；参数错误退出 2。图在一次完整标记前全部建好，没有周期中分配或修改。

## TODO 1：弱边必须在扫描时分类

模型的 W 同时有 `array[1]=A`、`map[K]=V`、`metatable=M`，另有不会指向 GC 对象的
`Atom→Atom` 条目。默认 roots 只有 W。先填写各对象存活预测：

| mode | 数组 value | map key | map value | metatable | 本例最终存活 |
| --- | --- | --- | --- | --- | --- |
| 普通 | strong | strong | strong | strong | W/K/V/A/M |
| `v` | weak | strong | weak | strong | W/K/M |
| `k` | strong | weak | key 已活才传播 | strong | W/A/M |
| `kv` | weak | weak | weak | strong | W/M |

`k` 的数组索引是非 GC key，数组 value 仍为 strong；`kv` 中即使 K 由另一 root 保活，
V 也不能因此变 strong。metatable 是单独的 strong 边，不随条目模式变弱。

初始 `classify_edges` 把全部子边放进标记队列。运行 `weak` 会看到错误版连 V/A 都
留下，弱清理找不到白对象。这不是“删除失败”；它们早在扫描时就被标为可达了。
请只修改这个函数，按表中规则返回拥有型 `Vec<Value>`，让 `k` 的 map value 留给
下一阶段判断。可选参考为 [solution.rs](solution.rs) 的同名函数。

```diff
- let mut edges = table.array.clone();
+ let mut edges = Vec::new();
+ if !table.mode.weak_values() {
+     edges.extend(table.array.iter().copied());
+ }
  for &(key, value) in &table.map {
-     edges.extend([key, value]);
+     if !table.mode.weak_keys() { edges.push(key); }
+     if !table.mode.weak_keys() && !table.mode.weak_values() { edges.push(value); }
  }
```

保留后面的 metatable 处理。上面的 `Atom→Atom` 不受 GC 管理，在四种模式下都留下，
所以 CLI 的弱表 map 长度为 1，而不是 0；真实 Heap 探针没有这条辅助条目。

```powershell
# 初始失败 1 项；完成 TODO 1 后通过 1 项。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_exercises weak_edges_follow_table_mode -- --ignored --exact

# 默认参考行为通过 1 项。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_reference weak_edges_follow_table_mode -- --exact
```

验收覆盖普通/v/k/kv、K 另有 root、primitive key、metatable 以及不可达 K↔V 环。
不能靠把 K/V 加进 roots 或跳过弱清理来通过。模型中的 `Value::Ref(id)` 是整数 handle；
Rust 复制它不会标记目标对象，依然要依靠标记队列。

## TODO 2：一轮传播为什么漏掉 V2？

固定输入为 roots=W1/W2/K1，`W1[K1]=V1`，V1 strong→K2，`W2[K2]=V2`。
W1/W2 都是 `k` 模式。初始 strong 扫描结束时，只有 W1/W2/K1 marked。

`candidates()` 用共享借用读取**整轮开始时**的标记，收集拥有型 value Vec；之后才结束
读取、逐项 enqueue 并统一 drain。这既延续短借用模式，也让结果不依赖表的遍历顺序。
因此第一轮只得到 V1；扫描 V1 后才标记 K2，下一轮才能从 W2 得到 V2。

```text
before one-round: live=["W1", "W2", "K1", "V1", "K2"] V2_accessible=false
after fixed-point: live=["W1", "W2", "K1", "V1", "K2", "V2"] V2_accessible=true
  candidates=["V1"]
  candidates=["V2"]
  candidates=[]
```

单轮错误版中 K2 已活，W2 条目不会因弱 key 删除，但 V2 没被标记、被模型删除，导致
条目留下 stale value。这个故障属于固定小模型的假设设计，**不是当前真实 Heap 的行为**。

只修改 `propagate`，将原有“收集→入队→drain”重复到 candidates 为空；不要在扫描
每张表时立刻 drain 来依赖碰巧有利的遍历顺序。每个非空轮至少标记一个尚未标记的
可达对象，输入图有限且周期中不增加对象，所以循环能到达固定点。

```diff
+ loop {
      let candidates = heap.candidates();
      heap.record_round(&candidates);
+     if candidates.is_empty() { break; }
      for value in candidates { heap.enqueue(value); }
      heap.drain(classify);
+ }
```

```powershell
# 初始失败 1 项；完成 TODO 2 后通过 1 项。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_exercises ephemerons_reach_a_fixed_point_without_bootstrapping_cycles -- --ignored --exact

# 默认参考行为通过 1 项。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_reference ephemerons_reach_a_fixed_point_without_bootstrapping_cycles -- --exact
```

合同同时要求解除 K1 root 后整个链可回收，以及只有 W root 时 K↔V 强环不能靠彼此
自举存活。候选判断需要从“已 marked 的 key”出发；不能把“value 能反向到达 key”
当成 key 已活。另有分别需要 1、4、16 个非空传播轮次的长链，防止把两轮硬编码为
固定上限。primitive key 在此模型里一直已活，不需要对象标记。

## 参考、故障有效性与真实 Heap

```powershell
# 10 项通过：2 共享参考合同、2 固定故障、6 真实 Heap 对照。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_reference

# 初始 2 项均失败、退出码非零；两题完成后均通过。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_exercises -- --ignored
```

默认 workspace 会忽略这两道未完成的 learner，`2 ignored` 不等于已完成。
[real.rs](real.rs) 直接使用真实 v0.14 `Heap`，独立验证：

- 普通/v/k/kv 的条目清理及不同 roots 下的存活；
- 两张 weak-key 表的传播链，并在移除 K1 root 后释放整条链；
- 没有 key root 的 K↔V 环不能存活；
- 两张表共用一个 metatable，在**完整周期之间**将 `__mode` 从 `v` 改为普通模式、
  再改回 `v`，两个依赖表都刷新。第一轮另给 value roots 防止它们提前被回收，第二轮
  撤去这些 roots 后仍保活，第三轮再变弱后两值都释放。

存活通过 `Heap::table_array_len` 实际访问每个 key/value/metatable，清理通过
`table_get`/`table_next_pair` 访问容器；弱数组还必须精确返回 `Nil`、长度为 0。
只比较 handle 或打印 object_id 不构成保活证据。
`shared_mode_between_cycles` 返回三个 true；含义是初始外部 roots 保活、普通模式保活、
再次弱化后成功回收，不是三个阶段对象始终存活。

## 范围与源码对应

[model.rs](model.rs) 用 BTreeMap 固定轨迹顺序，拥有四类表边、marked 集合和 gray Vec。
它只模拟标记、弱清理和删除未标记对象；不模拟真实增量阶段、预算、分配、write barrier、
finalizer、resurrection 或完整 Lua key/value 类型。它也不提供任意坏 learner 的终止保证。
故障重放本身是有限图和一次/固定点两种推进，不会无限递归。

真实 [lua_core/src/lib.rs](../../crates/lua_core/src/lib.rs) 的对应顺序是
`mark_one` 分类强边 → `atomic_phase` 反复收集 ephemeron values 并 drain →
`cleanup_weak_tables` → Sweep。真实 Atomic 在弱清理之前还有 finalizer 发现及保活；
本模型不含这一步，不证明 finalizer 产生的新可达性会再次做 ephemeron 固定点。
真实模式刷新会修改依赖表 flags，本实验只在两个完整周期之间切换；不能外推为
周期中任意切换都即时生效。真实周期中写弱边也可能触发 barrier，让新对象多活一轮，
所以所有反例都在 Pause 配置好图，再开始完整回收。

理解题：为什么 `k` 的 array value 是 strong，但 map value 要看 key？
实现题：分别完成两个 hook，并跑两条显式 learner 命令。
调试题：如果一轮版留下 K2 而回收 V2，先看“候选何时收集”，还是先改 cleanup？
完成时应能解释：清理无法撤销过强标记；拥有型候选 Vec 结束共享借用；固定点不能让
无根环自举；暂时保活与永久 rooting 不同。

本机验证日志写到 `target/teaching-v14-weak-2026-10-15/`，这是忽略产物，不随 clone
交付。永久复现入口是本 README、starter/solution/contracts 和例子源文件。
