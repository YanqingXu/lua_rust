# v0.14：沿三个问题实现增量回收

从仓库根目录运行。本包只研究普通强引用：先把完整回收拆成 step，再跨调用保存进度，
最后处理两步之间写入的新边。`model.rs` 是可运行的小模型，不是三份历史 VM 快照；
它复用真实 core 的 `GcPhase`、`GcStep`、`GcStats`，并用真实 `Heap` 独立对照。

## 先观察反例

```powershell
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example gc_problem_chain
```

也可在命令后加 `-- split`、`-- progress`、`-- barrier` 或 `-- real`，只观察一个问题。
CLI 固定重放模型的错误与修复设计，不调用 `starter.rs`；即使完成 TODO，它仍会展示
原始反例。退出码 0 只表示重放执行完成，正确性由下面的验收另行断言。

| 问题 | 修改范围 | 初始反例 | 修复后的观察 |
|---|---|---|---|
| 怎样中途交还控制权？ | `starter.rs::split_step` | 首次已经 Pause，work=8，completed=4/1/3 | 首次 Mark，work=1，completed=none |
| 下一次怎样继续？ | `starter.rs::resume_step` | 每次都重启，重复 Mark/gray=[B] | 八次完成同一周期，统计 4/1/3 |
| 新写入的 child 会不会漏扫？ | `starter.rs::connect` | A 已扫描后接入 B，B 被误收 | B 入队并在 Atomic 扫描，collected=0 |

`completed=4/1/3` 表示 before/collected/remaining，不是字节数。模型的 `collect` 返回
汇总 work 的 `GcStep` 以便展示；真实 `Heap::collect` 返回 `GcStats`，没有 work 汇总。
模型用 BTreeMap 稳定输出顺序，gray 与真实实现一样用 LIFO Vec，右端是下一次 pop。

## 三个 TODO 和共享验收

只修改 [starter.rs](starter.rs) 的三个 TODO；[contracts.rs](contracts.rs) 是验收规格。
不修改断言、模型、原有 GC 或真实快照的参数来掩盖故障。

```powershell
# 初始：3 项都失败，退出码非零。完成 TODO 后：3 项都通过。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_exercises -- --ignored

# 参考入口、反例有效性、真实 Heap 对照：8 项通过。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_reference
```

默认 `cargo test --workspace` 编译 starter，但显式忽略 3 个尚未完成的练习。
`0 failed; 3 ignored` 不表示你已经完成；需要自己运行第一条命令。
单项验收例如：

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_exercises saved_progress_finishes_the_same_cycle -- --ignored --exact
```

### 问题一：给 collect 换一个名字够不够？

初始图为 root→A→B→C，另有不可达的 D 自环。`split_step` 忽略预算调用完整 collect，
所以首次已经回收 D。先预测首次返回值，再观察：

```text
before collect-as-step: phase=Pause work=8 completed=4/1/3
after one step: phase=Mark work=1 completed=none
```

`first_step_yields_before_collection_finishes` 不仅检查完成信号，还要求首次只扫描 A，
B/C/D 全部仍存在。提示：使用哪个已有入口，才能返回一次推进而非完整周期？
对照 [reference/solution.rs](reference/solution.rs) 的 `split_step`，注意预算参数也要传入。

### 问题二：进度属于局部调用，还是持续存在的 Heap？

`resume_step` 调用 `step_restarting`，它在每次入口清掉 marks、gray、sweep IDs 和 cursor。
这模拟“把完整 collect 截成一小段，却仍重建局部进度”的错误。CLI 只打印前三次重复
状态；`saved_progress_finishes_the_same_cycle` 验收会在最多八次调用内要求完成。

正确的返回轨迹如下，每次 work=1：

```text
call 1  Mark    gray=[B] cursor=0
call 2  Mark    gray=[C] cursor=0
call 3  Atomic  gray=[]  cursor=0
call 4  Sweep   gray=[]  cursor=0   snapshot=[A,B,C,D]
call 5  Sweep   gray=[]  cursor=1
call 6  Sweep   gray=[]  cursor=2
call 7  Sweep   gray=[]  cursor=3
call 8  Pause   gray=[]  cursor=4   completed=4/1/3
```

提示：只在 Pause 开始新周期；同一 Heap 保存队列与游标。不要把预算改大、不要扩大
循环上界，也不要伪造 Some(stats)。验收还检查 A/B/C 存活、D 删除以及解除根后的回收。

### 问题三：已经扫描的 A，为什么不能自动保住新引用？

另起只含 A/B 的图，两者在周期开始前分配，root 只有 A。第一次 step 返回 Atomic 后
A 已扫描、B 未扫描；随后写入 A→B。错误入口绕过 barrier，只改边，不增加标记工作。
再次提交 root A 不会重扫已扫描的 A，因此 B 被删除：

```text
before raw-write: phase=Pause work=3 completed=2/1/1
before raw-write child_live=false
after barrier: phase=Pause work=3 completed=2/0/2
after barrier child_live=true
```

提示：`set_edge` 集中执行写入与 barrier；不要把 B 偷加到 roots，也不要在周期中
重新分配 B。真实 Heap 会对新分配对象运行 `barrier_new`，这会掩盖原来的遗漏。
`barrier_preserves_a_late_white_child` 同时检查保活和解除全部根后的回收，防止永久保活
来伪装修复。模型的 is_live(false) 不是 `HeapError::StaleHandle`；真实 Heap 必须实际
访问 child，不能只因为 `table_get(A,key)` 返回旧 handle 就认定 B 存活。

## 参考验收证明了什么？

[reference.rs](reference.rs) 的 8 项验收分三组：

- 3 项与 learner 共用的正确行为合同；
- 3 项确认错误设计确实首次做完、重复首步、误收 child；
- 2 项真实 Heap 对照：四对象八次轨迹与 full collect 相同、实际对象分类正确；
  table_set 能保住周期前已存在的 white child，且 child 实际可访问。

真实快照的既有规格还可以定向运行：

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml -p lua_core tests::incremental_and_full_collection_reach_the_same_result -- --exact
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml -p lua_core tests::write_barrier_keeps_a_white_child_added_to_a_marked_table -- --exact
```

模型只在 Pause 分配对象、使用固定 roots，不实现 weak、finalizer、upvalue、Lua API、
并发或宿主错误恢复。真实 Heap 的 Mark/Atomic 会重扫 roots，Sweep 不重扫；清扫 ID
快照在 Atomic 结束时建立。budget 只计工作条目：roots、整张表、Atomic 和阶段重置仍
可能做大量工作，`budget=1` 不保证恒定延迟，也不证明实时暂停上限。
