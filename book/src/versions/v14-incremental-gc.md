# v0.14 · 增量 GC、弱表与 Finalizer

<div class="lesson-meta"><span>分步 → 进度 → barrier → 短借用 → 事件 → 重入</span><span>workspace</span><span>checkpoint v0.14</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把一次完整遍历拆成可暂停、可回调的状态机</strong>
  <p>前置：v0.13 的 roots/edges、mark/sweep 与 Heap owner。先解决分步、保存进度和
  写入修复，再追问：扫描时怎样结束短借用、谁拥有待执行事件、回调重入和报错时谁负责
  保活与收尾。产出是三个强引用 TODO、一个编译器借用实验、三个回调 TODO；每次改动
  都用同一反例前后对照。weak/ephemeron 固定点留作扩展阅读。</p>
</div>

v0.13 的一次 `collect` 要做完本轮 mark/sweep 才返回。现在让 VM 在回收步骤之间继续
执行 Lua，就会依次遇到三个问题：**怎样交还控制权，怎样记住进度，怎样修复扫描之后
新增的边**。前三问讲强引用状态机，后四问沿着“扫描 → 交还事件 → 调用 Lua”继续
追踪 Rust 借用、事件所有权、GC roots 与错误路径。自动 checkpoint、weak table 和
resurrection 的完整语义保留在扩展阅读，不作为前半练习的前置知识。

配套代码分层验证：`exercises/model.rs` 是只含强引用的可运行小模型，用来替换
错误设计、观察确定性轨迹；真实快照仍在 `crates/` 中，另用真实 `Heap` 对照验收。
小模型不是三份历史 VM 快照，也不支持 weak/finalizer。它复用真实 core 的
`GcPhase`、`GcStep` 和 `GcStats`。`exercises/borrow/` 用 rustc 检验真实借用冲突；
`exercises/callbacks/` 从已排好的事件批次开始模拟重入，另以真实 Heap/VM 验证保活、
调用次数和错误后续执行。后者不模拟 Atomic、weak 或完整 Lua VM。

<a id="learning-predict"></a>
## 先预测再运行

先画 `root → A → B → C`，旁边还有一个不可达对象 D，所有对象都在周期开始前分配。
预测下面三个输出，再运行命令：

1. 给 `step` 传预算 1，但内部直接调用完整 `collect`，第一次调用是否已经完成？
2. 每次调用重新建 gray stack、重置游标，能否最终扫到 B/C 并回收 D？
3. 另起一张只有 A/B 的图，root 只指向 A。扫描 A 后才加入 `A.x = B`，B 会不会存活？

```powershell
$gcManifest = 'book/code/v14-incremental-gc/Cargo.toml'
cargo run --quiet --manifest-path $gcManifest --example gc_problem_chain
```

CLI 在同一次运行中并列展示错误和修复后的行为；预期错误被记录为实验结果，进程正常
退出表示重放执行完毕，不表示错误设计通过。章末行为验收单独检查正确性，学习者的
三个 TODO 初始会以非零退出码失败。CLI 固定重放前后设计，不读取 starter 的修改；
完成练习后它仍会展示原反例，练习完成以 `gc_exercises -- --ignored` 为准。
所有命令都从仓库根目录运行。

<a id="learning-code-map"></a>
## 代码阅读地图

1. [小模型](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/model.rs)：先只看对象边、step 和写入；
2. [三个错误入口](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/starter.rs) 与 [参考改动](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/reference/solution.rs)：每次替换一处设计；
3. 真实 core 的 `Heap`、`GcPhase`、`GcStep`：谁拥有跨调用状态，怎样表达未完成与完成；
4. `Heap::step` 的 Pause/Mark/Atomic/Sweep 分支，再看 `collect` 怎样循环调用 step；
5. `table_set → write_barrier → enqueue_value`：集中处理新增边；
6. 运行 `budget=1`/完整回收对照与 late child 保活验收；
7. `mark_one` 的 record 借用 → 拥有型子边 → `enqueue_value`，对照短借用实验；
8. `Heap::take_finalizers → Vm::gc_step_internal → invoke_callable`：先移走事件、
   再保护全批 roots、最后调用 Lua 并收尾。

## 问题一：怎样在回收尚未完成时交还控制权？

### 先让旧设计暴露问题

预算为 1，但入口只是给完整回收换了个名字：

```diff
- heap.collect(roots)       // 一次做完；调用者没有机会在中途继续执行
+ heap.step(roots, budget) // 返回本步结果，下次还能继续
```

这是实验中 `split_step` 的核心替换，完整函数见 starter/reference。错误版本的第一次
调用已经回收 D；修复后第一次只扫描 A，`completed` 仍为 None。

错误入口（可以直接编译运行）：

```rust
{{#include ../../code/v14-incremental-gc/exercises/starter.rs:exercise_split}}
```

修复后的同一入口：

```rust
{{#include ../../code/v14-incremental-gc/exercises/reference/solution.rs:solution_split}}
```

```powershell
cargo run --quiet --manifest-path $gcManifest --example gc_problem_chain -- split
```

预期关键输出中，`completed` 的三个数字依次是 before/collected/remaining：

```text
before collect-as-step: phase=Pause work=8 completed=4/1/3
after one step: phase=Mark work=1 completed=none
```

这里 work=8 是小模型 `collect` 汇总的整轮工作量；真实 `Heap::collect` 返回 GcStats，
没有这个 work 汇总字段。比较的是“一次做完”与“一次交还控制权”的可观察差别。

问题不在函数名。只要内部仍循环到周期结束，就仍没有交还控制权的中间点。真实快照中，
`Heap::step` 一次只执行一个 phase 分支，而完整 `collect` 是它的驱动者：

```rust,ignore
loop {
    let step = self.step(roots, upvalue_roots, usize::MAX)?;
    if let Some(stats) = step.completed {
        return Ok(stats);
    }
}
```

这段与真实 `collect` 对应。Rust 的 `Result` 表达操作是否失败，`completed: Option<GcStats>`
表达本步成功后整个周期是否完成：`Ok + None` 是正常进度，不应被当作失败。

### 给每个返回点定义状态

| Phase | 本阶段工作 | 本次调用可能返回什么 |
|---|---|---|
| `Pause` | 显式 step 开始新周期；自动调度则先判断是否到 checkpoint | 同一次调用进入 Mark，开始处理 |
| `Mark` | 至多弹出 budget 个 gray 条目，枚举它们的边 | 仍为 Mark，或 gray 为空后变 Atomic；未完成 |
| `Atomic` | 完成一致性边界，最后生成排序的 sweep IDs | 变为 Sweep；未完成 |
| `Sweep` | 至多检查 budget 个 snapshot ID，删除其中 white 对象 | 仍为 Sweep，或回到 Pause 并返回 Some(stats) |

`GcStep.phase` 是执行后的阶段。Mark 返回 Atomic 不表示 Atomic 已经执行；Atomic
返回 Sweep 时，才刚准备好清扫快照。

**预算边界：** 本快照把 `budget=0` 归一为 1；Mark 的重复队列条目也计一次 work，
Sweep 的存活 ID 也计一次 work。启动周期的全堆重置、roots 扫描、单个大表的边遍历、
整个 Atomic 和结束时重置 marks 都可能做额外工作。因此预算是对象/条目的工作计数，
不能解释成每次最多 1 毫秒或恒定暂停时间。小模型保留这个接口含义，不提供延迟保证。

```powershell
cargo test --manifest-path $gcManifest --test pipeline explicit_gc_step_reports_completion_without_auto_step_stealing_it -- --exact
```

**完成条件：** 能区分“本次 step 返回”和“本轮 GC 完成”，指出完整驱动循环该放在哪里。
接着会遇到下一问：函数已经返回，gray stack 和游标由谁保管？

## 问题二：下一次调用怎样接着上次的位置做？

### 每次重新开始，预算越小反而越做不完

`resume_step` 的错误版本在每次调用前都重启周期。局部工作队列随调用结束而丢失，
下一次又把 A 当成未访问对象。给循环加“最多 8 次”的边界，观察它反复停在相同位置：

```powershell
cargo run --quiet --manifest-path $gcManifest --example gc_problem_chain -- progress
```

```diff
- heap.step_restarting(roots, budget) // helper 内部无条件重置周期
+ heap.step(roots, budget)            // 只有 Pause 才启动新周期
```

这两个调用来自以下可运行入口，改动无需碰模型的 mark/sweep 实现：

```rust
{{#include ../../code/v14-incremental-gc/exercises/starter.rs:exercise_resume}}
```

```rust
{{#include ../../code/v14-incremental-gc/exercises/reference/solution.rs:solution_resume}}
```

删除无条件 restart 的理由是“进度必须属于一个持续存在的 owner”。不是永远不重置，
而是只在 Pause 开始新周期时重置。真实快照将需要跨调用的局部状态移进 `Heap`：

```rust,ignore
pub struct Heap {
    // objects、heap_id 等已有字段省略
    phase: GcPhase,
    gray: Vec<TraceRef>,
    sweep_ids: Vec<u64>,
    sweep_cursor: usize,
    cycle_before: usize,
    cycle_collected: usize,
    // allocation / finalizer 等扩展字段省略
}
```

`Vec` 拥有队列，`usize` 游标记录已检查多少 ID；它们都由 Heap 保存，不需要持有上一
次函数调用的局部引用。`GcPhase` 让 `match` 明确选择本步工作，但这些字段仍需靠运行时
不变量共同维护，enum 本身不会自动排除“错误游标配上错误 phase”的全部组合。

![A 到 B 到 C 的回收进度跨调用保存；错误版本每次重启只扫描 A，正确版本最终清扫 D。](../images/v14-step-progress.svg)

### 在同一张图上手推八次调用

gray 是 LIFO stack，下表右端是下一次 pop；A/B/C/D 按分配顺序取 ID。

| 返回次数 | 执行后的 phase | gray | sweep cursor | 本次效果 |
|---|---|---|---|---|
| 1 | Mark | [B] | 0 | A 已扫描，返回调用者 |
| 2 | Mark | [C] | 0 | 接着扫描 B |
| 3 | Atomic | [] | 0 | C 已扫描，等待下一次执行 Atomic |
| 4 | Sweep | [] | 0 | Atomic 完成，生成 [A,B,C,D] 清扫快照 |
| 5 | Sweep | [] | 1 | 检查并保留 A |
| 6 | Sweep | [] | 2 | 检查并保留 B |
| 7 | Sweep | [] | 3 | 检查并保留 C |
| 8 | Pause | [] | 4 | 删除 D；返回 before=4、collected=1、remaining=3 |

模型与真实 Heap 使用相同的四对象图作对照。真实 `Heap::step` 的 Mark/Atomic 会
重新播种当前 roots，但只 enqueue 尚未扫描的 root；Sweep 不重扫 roots。若把已扫描的
A 反复压回 LIFO 顶部，预算 1 也会被重复工作耗尽。跨调用保存队列与正确播种根两件事
都要满足。

CLI 的错误分支只打印前 3 次重复状态，独立验收会检查完整的 8 次上界。修复分支的
末行应为 `after call=8: phase=Pause work=1 completed=4/1/3`。Pause 中保留的对象已
清掉 mark，诊断输出会重新显示 white；这表示准备下一周期，不表示本轮误删了存活对象。

```powershell
cargo run --quiet --manifest-path $gcManifest --example gc_problem_chain -- real
cargo test --manifest-path $gcManifest -p lua_core tests::incremental_and_full_collection_reach_the_same_result -- --exact
```

**完成条件：** 修复版在有界调用次数内完成；与完整回收得到相同统计，而且 A/B/C
实际可访问、D 实际被删除。只比较一个完成布尔值不够，也不能用无限 while 掩盖无进展。
一旦 VM 能在这些返回点继续执行，第三个问题就出现了：对象边会在两步之间改变。

## 问题三：已经扫描的 parent 又指向白对象，怎样修复？

### 固定“先扫描，再写入”的交错顺序

另建一张只有 A/B 的图，root 只指向 A，B 在周期开始前已经分配。第一次 step 扫描 A
并返回 Atomic，此时 A 已扫描、B 未扫描。然后才写入 `A.x = B`。

```powershell
cargo run --quiet --manifest-path $gcManifest --example gc_problem_chain -- barrier
```

遗漏 barrier 的版本只更新容器。Atomic 重扫 root A 时发现 A 已扫描，便不重新遍历
它的字段；B 没有进入待处理队列，Sweep 随后删除 B。对象图里留下指向已回收 B 的边。

```diff
- heap.set_edge_without_barrier(parent, child);
+ heap.set_edge(parent, child);
```

完整的前后入口分别是：

```rust
{{#include ../../code/v14-incremental-gc/exercises/starter.rs:exercise_barrier}}
```

```rust
{{#include ../../code/v14-incremental-gc/exercises/reference/solution.rs:solution_barrier}}
```

以上是小模型中 `connect` 的替换；真实 `Heap::table_set` 在写入前校验 key/value，
分别调用 `write_barrier`，随后修改 table。核心判断是：owner 已扫描，且 phase 仍为
Mark/Atomic，就把尚未扫描的 child 放入 gray。B 在本例的 Atomic 内被扫描，然后才能
建立 Sweep 快照。

```text
before raw-write: phase=Pause work=3 completed=2/1/1
before raw-write child_live=false
after barrier: phase=Pause work=3 completed=2/0/2
after barrier child_live=true
```

`table_get(parent, key)` 返回一个 handle 本身不能证明 child 存活；真实 Heap 验收还要
用 `is_live(child)` 或实际访问 child 检查。模型中 `child_live=false` 明确指出误回收，
它不会伪装成真实 Heap 返回的 `StaleHandle` 错误类型。

![扫描 A 后插入到 B 的新边：遗漏 barrier 会误收 B，集中写入时 enqueue B 则能保留。](../images/v14-write-barrier.svg)

```rust
{{#include ../../code/v14-incremental-gc/crates/lua_core/src/lib.rs:write_barrier}}
```

图中的 white/gray/black 是教学上的三种角色。真实对象只有 `marked: bool`，并用
`gray: Vec<TraceRef>` 记录待处理工作：`mark_one` 在 pop 后才设置 marked。B 入 gray
时并没有变成一个名为 Gray 的 Rust enum variant。用“加入待处理队列”解释操作，才能
把图与源码对应起来。

本例选择周期前已分配的 B，是为了确保测到写入 barrier。Mark/Atomic 中**新分配**
对象会经过 `barrier_new`，可能掩盖遗漏写入 barrier 的故障；Sweep 中的新对象则标记
为存活，且不属于 **Atomic 结束时** 已建立的 sweep snapshot。

```powershell
cargo test --manifest-path $gcManifest -p lua_core tests::write_barrier_keeps_a_white_child_added_to_a_marked_table -- --exact
```

**完成条件：** 同一交错顺序下，错误版让 B 不可访问，修复版保留 B；根集合中始终
只有 A，不能把 B 偷偷添加为直接 root 来通过测试。说明这条补丁修复的是新增引用的
追踪，不是增加一次分配，也不是每步重新扫描所有已标记对象。

## 三个问题怎样映射到真实快照？

| 小模型中的改动 | 真实 Rust 实现 | 保持的不变量 |
|---|---|---|
| 完整 collect 驱动与 step 分开 | `Heap::collect` 循环调用 `Heap::step` | 成功返回可未完成，只有 Some(stats) 表示周期结束 |
| 只在新周期重置，保存队列和游标 | `Heap` 字段与 `start_cycle`，由 `GcPhase` 决定调用时机 | 每步从上次进度继续，不丢工作、不无限重启 |
| 集中写入并 enqueue 新 child | `table_set → write_barrier → enqueue_value` | Mark/Atomic 中已扫描 owner 的新引用不会漏扫 |

[完整 core 源码](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/crates/lua_core/src/lib.rs)
还覆盖 table key、metatable、upvalue、userdata 等 mutation family。三个练习只证明
上面的强引用案例；不能把模型验收外推为全部写入路径、weak table 或 finalizer 已验收。

## 问题四：借着 record，为什么不能继续修改 Heap？

### 编译通过前，先让编译器指出冲突

扫描 parent 时，我们已借用 `self.records[id]`，却还要调用需要 `&mut self` 的
`enqueue`。先预测：把以下循环写进 `mark_one`，rustc 会接受吗？

```rust,ignore
{{#include ../../code/v14-incremental-gc/exercises/borrow/before.rs:long_borrow_before}}
```

循环每轮还要读取 `record.fields`，由 Heap 派生的可变借用仍在使用；调用整个 Heap 的
可变方法就与之重叠。这里应观察到 **E0499**，不是 GC 运行时报错。反例作为独立
rustc 输入保存，不是 Cargo 自动编译的 target；下面两项默认验收分别检查编译失败
的错误码，以及修复后的编译和运行结果。

```powershell
cargo test --manifest-path $gcManifest --test gc_borrow_reference -- --nocapture
```

这条命令通过表示“错误版确实被拒绝，修复版行为正确”。如果把第二次调用改成只修改
互不相交的字段，编译器有时能拆分字段借用；本例的冲突来自 `enqueue(&mut self, …)`
借用整个 Heap，不能概括成“借过一个字段就不能改其他任何字段”。

### 先拥有待处理子边，再结束 record 借用

我们需要离开 record 后仍能使用子边，所以应提取 `Vec<Value>`，而非仍指回 record
的 `Vec<&Value>`。最小差异是把读取与队列修改分开：

```diff
- for value in &record.fields { self.enqueue(value.clone()); }
+ let children = { /* 借用 record、设置 marked */ record.fields.clone() };
+ for value in children { self.enqueue(value); }
```

可编译的完整方法如下：

```rust
{{#include ../../code/v14-incremental-gc/exercises/borrow/solution.rs:short_borrow_solution}}
```

Rust 的 NLL（非词法生命周期）在最后一次使用后结束借用；显式块使边界容易看见，
不需要 `drop(&mut record)`。`clone` 复制 Value 和 handle，不复制目标 GC 对象，也
不自动把对象加入 roots。此时只解决了“工作项离开借用后还可使用”，保活还要由后续
入队、扫描和真实 GC roots 完成。

![长借用跨过 enqueue 导致冲突；提取拥有型子边后，record 借用结束，才能重新借用 Heap。](../images/v14-short-borrow.svg)

验收要求 parent 扫描后 `gray=[1, 2]`，继续排空后 parent 与两个 child 都 marked，
不可达自环仍未标记。删掉循环虽然可能消除 E0499，却不能通过行为验收。

真实 `Heap::mark_one` 将 table 条目、closure/upvalue 或 userdata 的待追踪值提取成
本地 `values` / `references`，在 record 最后一次使用后执行 `self.gray.extend` 与
`self.enqueue_value`。它还处理 weak 分类和访问错误；四对象借用模型没有这些合同。

**完成条件：** 只改 `borrow/starter.rs::mark_one`，让显式学习者测试从 E0499 失败变为
编译且行为通过。随后考虑：GC 扫描结束，要把 finalizer 交给能再次调用 GC 的 Lua，
还可以借着 Heap 内部队列遍历吗？

## 问题五：事件交给 VM 后，为什么必须从 Heap 队列移走？

### 拥有一份 clone，原队列仍然在那里

本节从已发现的两个 finalizer 事件 A/B 开始。事件只包含 userdata/callback 的 handle。
先预测：VM 克隆事件 Vec，再执行 A；A 重入回收并再次取队列，是否还会取到 A/B？

```powershell
cargo run --quiet --manifest-path $gcManifest --example finalizer_problem_chain -- ownership
```

错误版只允许**总共一次重入**，因此不会无限递归；仍会得到 `calls=[1, 1, 2, 2]`，
且源队列 `pending=2`。错误来自重复取到同一批事件，不能靠增加递归上限来修复。

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/starter.rs:callback_take_starter}}
```

只替换事件交接这一处：

```diff
- pending.clone()
+ std::mem::take(pending)
```

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/solution.rs:callback_take_solution}}
```

`mem::take` 移走旧 Vec，并在原位置留下空 Vec；返回值拥有整批事件，函数返回后对
Heap 的借用已经结束。执行 A 时重入只会看到空的源队列，于是本例变为 `calls=[1, 2]`、
`pending=0`。真实 `Heap::take_finalizers` 使用相同操作，VM 随后才调用需要
`&mut self` 的 `invoke_callable`。

Atomic 在发现事件时先设 `finalized=true`，防止以后周期再次**发现**同一 userdata；
移走 ready 队列防止当前事件再次被**取出**。两处解决不同的问题，不能互相替代。

```powershell
cargo test --manifest-path $gcManifest --test gc_callbacks_reference transferred_batch_is_invisible_to_reentry -- --exact
```

**完成条件：** 源队列在第一个 callback 前已经为空，重入不会重复执行当前批次。
但这份拥有型 Vec 里只有 GC handle；下一问仍然必须回答它们在嵌套 GC 中怎样存活。

## 问题六：拥有事件 Vec，为什么仍会误收尚未执行的兄弟事件？

### 当前调用帧只能保护当前事件

沿用 A/B，改回正确的 `take`，只移除“全批临时 roots”。A 正在执行时，其活动帧能
保护 A 的 callback/参数；B 还没有调用帧。A 发起下一轮 GC，collector 不会自动扫描
Rust 栈中所有 Vec，B 的 userdata 与 callback 便可能消失。

```powershell
cargo run --quiet --manifest-path $gcManifest --example finalizer_problem_chain -- roots
```

错误版得到 `calls=[1]` 和 `Err(Stale(2))`；它仍拥有 B 的事件，却不能继续访问 B。
这是独立模型的错误类型，不是伪造真实 Heap 的诊断。修复必须在**第一个回调之前**
保护整批事件：

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/starter.rs:callback_pin_starter}}
```

```diff
- // 只依赖当前 invoke 的活动帧；没有保护尚未调用的事件
+ for event in events {
+     roots.push(event.callback);
+     roots.push(event.userdata);
+ }
```

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/solution.rs:callback_pin_solution}}
```

真实 VM 的 `gc_roots` 包含 `temporary_roots`。`gc_step_internal` 先记录旧长度
`root_base`，再把整批 callback 和 userdata 的 Value 都推入，最后才逐个调用。图中
“Rust 事件所有权”和“collector 的可达根”是两个维度：

![mem::take 转移事件批次；全批临时 roots 独立保护 userdata 和 callback，使 A 重入时 B 仍可访问。](../images/v14-finalizer-batch.svg)

### 用真实访问证明保活

```powershell
cargo test --manifest-path $gcManifest --test gc_callbacks_reference real_heap_owned_vec_requires_explicit_roots -- --exact
cargo test --manifest-path $gcManifest --test gc_callbacks_reference real_vm_reentrant_finalizer_keeps_sibling_alive_and_runs_once -- --exact
```

第一项保留真实 `Heap::take_finalizers` 返回的 Vec，再启动完整下一周期。仅 root 当前
事件时，B 的 `userdata_is_finalized` 与 `function_proto` 都访问失败；root 全批时两者
都成功。检查 object ID 只能证明 handle 中还有数字，不能证明对象仍存在。

第二项运行以下真实 VM 输入（Rust 包装只负责 compile/execute）：

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/real.rs:callback_real_pair}}
```

A 内调用两次 `gc_collect`，避免一次只完成外层剩余 Sweep 而没有真正进入下轮的
假阳性。B 的 callback 仍能调用，并用 `setfinalizer(value, nil)` 实际访问 userdata；
最后序列必须为 **123**，从而同时证明 A 重入返回后继续执行、B 存活且每项只调用一次。
这是同步重入，没有涉及线程或 async。

**完成条件：** 小模型从 stale 变为 `calls=[1, 2]` / `Ok(())`，真实 Heap 的两个对象
访问都成功，真实 VM 得到 123。下一问是：已保活整批事件，如果 A 报错，B 和临时
roots 又由谁收尾？

## 问题七：第一个 callback 报错，为什么不能立即用 `?` 返回？

### 提前传播错误会同时跳过事件与清理

先预测 A、B、C 三个事件，A/B 都报错，C 成功。外层已有一个 root 900，drain 前
追加全批 roots。下面的 `?` 会使调用序列、最后的 roots 分别变成什么？

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/starter.rs:callback_drain_starter}}
```

```powershell
cargo run --quiet --manifest-path $gcManifest --example finalizer_problem_chain -- errors
```

错误版只得到 `calls=[1]`，返回 `Callback(1)`，全批 roots 还留在 Vec 中。下一次 GC
仍保留本应可回收的兄弟对象；只检查“返回了错误”会漏掉这两处副作用。

```diff
- vm.invoke(event)?;
+ if let Err(error) = vm.invoke(event) {
+     if first_error.is_none() { first_error = Some(error); }
+ }
  // 遍历完剩余事件后
  vm.truncate_roots(root_base);
- Ok(())
+ first_error.map_or(Ok(()), Err)
```

完整修复如下，重点是先保存长度、继续 drain、恢复长度，再返回首错：

```rust
{{#include ../../code/v14-incremental-gc/exercises/callbacks/solution.rs:callback_drain_solution}}
```

![A 报错后保存首错，B、C 继续执行，最后恢复外层 roots 并返回 A 的错误；提前问号会跳过后续。](../images/v14-finalizer-errors.svg)

修复后必须得到 `calls=[1, 2, 3]`，仍返回 `Callback(1)`，roots 精确恢复为 `[900]`。
这里不能 `clear()`，否则会删除外层调用的 roots；也不能用 B 的错误覆盖 A。后续 GC
可以回收无其他 roots 的兄弟对象，说明清理没有泄漏。

```powershell
cargo test --manifest-path $gcManifest --test gc_callbacks_reference callback_error_drains_and_restores_roots -- --exact
cargo test --manifest-path $gcManifest --test gc_callbacks_reference real_vm_callback_error_still_executes_the_sibling -- --exact
```

真实 VM 对照中，A 触发 Type 错误，但 B 仍设置 `sibling_after_error=true`；随后在
**同一个 VM** 执行探针读取该值。真实实现也在 `gc_step_internal` 中保存首错并清理
temporary roots；但重新 `execute` 本身会清空临时 roots，所以该探针只证明错误类型
和兄弟回调已执行，精确清理由模型合同与真实源码中的 `truncate(root_base)` 核对。
这一协议处理这里的 `Result::Err` 路径，没有承诺 panic/abort 时也
执行相同清理；callback 还可能推进 GC，因此外层先前捕获的 step 结果不代表回调后的
即时 phase。

**完成条件：** 调用次数、首错、外层 roots 保留、后续对象可回收四项一起成立。
这些实验验证事件消费协议；weak/ephemeron 与 resurrection 仍需各自的具体反例。

## 扩展阅读：自动调度、弱表与 resurrection

以下内容帮助定位最终 v0.14 已有能力。第一遍可以先完成章末练习，再回来阅读；
完整 weak/ephemeron 教学尚未拆成问题链。

## Allocation accounting 与自动 checkpoint

Heap 为每次 Table、Closure、Upvalue、Userdata 分配累计 debt。达到阈值，或一个周期已经处于活动 phase，VM 会在每条指令完成并写回寄存器后推进少量工作。checkpoint 选在写回之后，避免刚创建但尚未进入 root 的对象被误回收。

`with_gc_allocation_threshold(1)` 的压力测试在 100 次循环分配中让每个安全点推进 GC，同时验证寄存器、循环清理和 barrier 不产生 stale handle。周期完成后 debt 归零；v1.0 保留教学用对象计数合同，并在 `LUA51-D010` 中显式登记与官方字节计账/调参接口的差异。

## Weak value、weak key 与 ephemeron

Table 在设置 metatable 时读取 `__mode`：包含 `v` 时不从 table 标记 value，包含 `k` 时不直接标记对象 key。之后修改 metatable 的 `__mode` 会刷新所有依赖 table 的模式。

Weak value 可以在 Atomic 直接依据 mark bit 删除。Weak key 更微妙：若 key 从其他 root 可达，那么它对应的 value 必须保留；value 又可能使其他 weak key 可达，所以需要重复到固定点：

```text
key 已标记 → 标记 value → 传播新 edges → 再检查 ephemeron
```

Atomic 先反复执行“已标记 key → 标记 value → 传播 edges”到固定点，再发现并排队
finalizer、重新标记相关 userdata/callback 并排空 gray，最后清理弱边并建立 Sweep 快照。
[查看完整 Atomic/weak/finalizer 实现](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/crates/lua_core/src/lib.rs)

固定点完成后，weak key/value 中指向未标记 heap object 的 entry 被删除。Primitive key/value 不由 GC 管理，因此视为存活。数组部分在 weak-value 模式下把死亡元素改为 nil，再执行尾部收缩。

## Finalizer、resurrection 与 second death

本版加入最小 `UserdataRef`。`setfinalizer(userdata, callback)` 只是教学入口；Atomic 按 object ID 升序发现“white、尚未 finalized、拥有 callback”的 userdata：

1. 设置 `finalized = true`，本周期重新标记 userdata 和 callback；
2. 把事件交还 VM，在 Heap 可变借用结束后调用 Lua callback；
3. callback 可把 userdata 写入 global/registry，形成 resurrection；
4. 以后失去所有 roots 时，因 `finalized = true` 不再排队，直接在下一周期死亡。

Object ID 顺序让教学测试可重复，但不是 v1.0 要宣称的 Lua 5.1 finalizer 顺序合同。正式兼容收口时必须与官方 oracle 对照并登记差异。

问题五至七已单独验收事件转移、整批保活和错误后继续执行；这里的 resurrection 则是
callback 主动把对象发布到新的长期 root。当前 VM 中 callback 报错后允许调用者再次
执行并继续推进 GC，不能因此认为报错时周期已经全部完成。

## Rust 知识映射与设计选择

- `GcPhase` enum 把 collector 控制流变成显式、可测试状态机；
- 持久化 gray/sweep cursor 让一次函数调用不再等于一个完整周期；
- `allocation_threshold` 与 saturating debt 表达自动调度策略；
- write barrier 集中在唯一 owner 的 mutation API，减少漏点；
- `HashSet<object_id>` 是 Atomic 弱引用清理时的稳定 mark snapshot；
- finalizer event 先脱离 Heap 借用再回调 VM，允许安全重入；
- “记录第一个错误、继续 drain”让失败恢复成为显式协议。

替代方案是 incremental update barrier：把 black parent 重新染 gray，而本版采用 insertion barrier 直接标记新 child。两者都能维护三色不变量；直接 child barrier 更适合当前无并发、不可移动 arena。Atomic 也可以按预算细分，但 weak/finalizer 固定点作为不可暂停边界更容易先证明正确。

## 从 v0.13 迁移了什么

- 重构：一次性 `mark_from + retain` 变为四阶段状态机；
- 新增：gray stack、sweep cursor、周期统计与单步预算；
- 新增：所有 Heap 写入路径的 barrier 与新分配着色规则；
- 新增：allocation accounting、自动 VM checkpoint 和阈值压力测试；
- 新增：`__mode` weak key/value、ephemeron 固定点和 Atomic 清理；
- 新增：最小 Userdata、finalizer 队列、resurrection、second death、重入和失败恢复；
- 保留：v0.13 typed handle、foreign/stale 校验、roots 与确定性 shutdown；
- 没有提前加入：coroutine、标准库适配器或正式 userdata/debug API。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml -- `
  book/code/v14-incremental-gc/examples/demo.lua
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --workspace
```

示例用 `gc_step(1)` 完成一个增量周期，清掉 weak value、运行并 resurrect userdata，随后解除 root 验证 second death：

```text
type=number value=714
```

基础快照的 84 项测试仍保留：59 个端到端测试、7 个编译器测试、15 个 core 测试和
3 个 VM 测试。三个强引用问题另有 8 项参考验收；本次短借用新增 2 项、事件与重入新增
9 项参考验收，默认 workspace 合计 **103 项通过、7 项学习者 TODO 显式 ignored**。
默认通过说明反例检测、参考合同与原有回归通过，不表示七个 TODO 已完成。

## 本版本暂不支持

- Atomic 仍是 stop-the-world 固定点，不按预算拆分；
- debt 按对象数而非真实 allocator bytes 统计；
- userdata 只有身份和 finalizer，没有 payload、完整 metatable 或宿主资源析构器；
- 教学 helper 不是 Lua 5.1 `collectgarbage` / debug / userdata 公共 API；
- 没有 coroutine，尚未证明 collector 与 yield/resume 的 owner turn；
- 没有标准库、Lua 模块或 CLI 工具完善。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>预测预算 1 的首次调用是否完成、A/B/C/D 仍有哪些存活；修复 starter.rs 的
  <code>split_step</code>，让首次调用只做一步。解释为什么把函数改名为 step
  不会自动产生交还控制权的中间点。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>修复 <code>resume_step</code>，让同一个 Heap 在至多 8 次 budget=1 调用中完成
  A→B→C、D 自环的周期。打印每次阶段、队列与游标，并与 full collect 对照；
  不要增大预算或循环上界来隐藏重启错误。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>修复 <code>connect</code>，让已扫描 A 在两步之间新接入的 B 存活。保持唯一 root
  为 A、保持 B 在周期前分配；比较遗漏 barrier 与正确写入的结果，再验证解除所有 roots
  后两者都能被回收。</p>
</div>

### 修改范围、运行命令和成功条件

只修改 [starter.rs](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/starter.rs)
的三个 TODO。[实验说明](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/README.md)
提供逐步提示与运行输出；[共享合同](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/contracts.rs)
固定验收条件。不要修改合同断言，也不要更改真实 GC 来配合错误输出。

```powershell
# 初始 3 项失败；修复三个 TODO 后应有 3 项通过。
cargo test --manifest-path $gcManifest --test gc_exercises -- --ignored

# 可选参考解、故障有效性与真实 Heap 对照。
cargo test --manifest-path $gcManifest --test gc_reference
```

| 验收名 | 初始反例 | 修复后必须观察到 |
|---|---|---|
| `first_step_yields_before_collection_finishes` | 首次就完成，work 超过 1，D 已回收 | Mark/work=1/completed=None，A 已扫描，D 仍存在 |
| `saved_progress_finishes_the_same_cycle` | 每次重启后停在相同 Mark，8 次仍未完成 | 经 Atomic/Sweep 完成；统计 4/1/3，A/B/C 存活、D 消失 |
| `barrier_preserves_a_late_white_child` | A 新引用的 B 没有入队，被 sweep | B 存活；随后移除所有 roots，A/B 都能回收 |

每个验收可以独立执行，例如：

```powershell
cargo test --manifest-path $gcManifest --test gc_exercises saved_progress_finishes_the_same_cycle -- --ignored --exact
```

starter 中的错误设计能编译，因此用 ignored 将它们与默认回归分开；显式启用时必须
看到失败，不能把“0 failed、3 ignored”作为练习完成证据。参考 target 也专门检查这三种
故障确实发生，并另测真实 Heap 的相同四对象轨迹和真实 table 写入保活。

模型不进行周期中分配，不提供 Lua 标准库、并发或可恢复 OOM 合同。其根集合在示例中
固定，节点是简单强引用图。真实 Heap 使用任意字节 Value、typed handle 和更多对象
种类；若要扩大到这些路径，应另建具体反例和验收，不能直接外推本轮结果。

### 短借用、事件与重入的四项验收

理解题：先解释 `Vec<&Value>` 为什么仍会借着 record，再只改
`exercises/borrow/starter.rs::mark_one`，让编译与子边扫描同时通过。不要修改固定的
`before.rs`、compiler 检测器或行为断言。

实现题：只改 `exercises/callbacks/starter.rs` 的 `take_batch` 与 `pin_batch`。
先移走源队列，再在任一 callback 前保护全批；不能靠禁止重入、增加长期全局 root 或
直接删除事件通过验收。

调试题：只改同一 starter 的 `drain_batch`。保留外层 root，继续执行报错项之后的
事件，恢复临时 roots，再返回首错。合同分别注入一个坏 hook，其他 hook 使用参考
实现，因此每题都可以独立修复。

```powershell
# 初始分别 1 项失败、3 项失败；完成后必须全部通过。
cargo test --manifest-path $gcManifest --test gc_borrow_exercises -- --ignored
cargo test --manifest-path $gcManifest --test gc_callbacks_exercises -- --ignored

# 固定反例、参考改动与真实 Heap/VM 对照，合计 11 项通过。
cargo test --manifest-path $gcManifest --test gc_borrow_reference --test gc_callbacks_reference
```

| 学习者验收 | 修改位置 | 完成后必须观察到 |
|---|---|---|
| `short_borrow_scans_owned_edges` | borrow 的 `mark_one` | 编译通过；两个 child 入队并扫描，不可达自环仍未标记 |
| `transferred_batch_is_invisible_to_reentry` | callbacks 的 `take_batch` | A/B 各执行一次，源 pending 为空 |
| `whole_batch_roots_protect_siblings` | callbacks 的 `pin_batch` | A 重入时 B 的 userdata/callback 都存活，结束恢复外层 roots |
| `callback_error_drains_and_restores_roots` | callbacks 的 `drain_batch` | A/B 报错仍执行 C、返回 A 首错、仅保留外层 roots、后续可回收 |

[短借用说明](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/borrow/README.md)
和 [回调说明](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/exercises/callbacks/README.md)
提供独立复现命令、可选参考解和实验边界。两个 CLI 都固定重放错误与修复，退出码 0
只表示重放完毕；学习者完成状态由上述 `--ignored` 验收决定。短借用题在 rustc 子进程
内编译，因此保留初始 E0499 不会让 workspace 构建失败。

<a id="learning-checkpoint"></a>
## 完成检查点

- 能从“首次做太多”推导出 collect 驱动与 step 的分工；
- 能指出每个返回点之后，phase、gray、sweep IDs、cursor 分别由谁保存；
- 能用八次 trace 解释为什么每次重建进度无法收敛；
- 能先扫描 A、再写入 B，复现并修复遗漏 barrier 的误回收；
- 能区分概念颜色、真实 marked bool、待处理队列，以及 step 的完成信号；
- 能让 record 借用先结束，并说明拥有型子边不等于 GC root；
- 能分开解释 `finalized`、`mem::take`、全批 temporary roots 的作用；
- 能手推 A 重入时源队列、B 的可达性、首错和 root_base 的变化；
- 七项学习者验收显式运行通过，并能解释小模型、编译器和真实 Heap/VM 各自验证的范围。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 collector phase、gray list、barrier 与 weak/finalizer 队列是本版状态机的性能化目标；
- [Table、所有权与共享对象](../ch02-runtime/table.md) 可对照 weak edges 为什么不能用 Rust 所有权直接表达；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可定位安全 checkpoint 为什么放在指令写回之后；
- 下一 checkpoint 是 v0.15：让 coroutine 拥有独立栈与调用帧，并验证 yield/resume 跨越 GC phase 时的 owner turn。
