# v0.14 · 增量 GC、弱表与 Finalizer

<div class="lesson-meta"><span>问题链：分步 → 保存进度 → 写入修复</span><span>workspace</span><span>checkpoint v0.14</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把一次完整遍历拆成可暂停状态机</strong>
  <p>前置：v0.13 的 roots/edges、mark/sweep 与 Heap owner。本轮先解决三个问题：怎样分步
  回收、下一次调用怎样接着做、步骤之间写入新引用怎样避免误收。沿问题引入 enum/match、
  持久 Vec/游标、Option 完成信号与封装写入；每个改动都由同一个反例前后对照验收。</p>
</div>

v0.13 的一次 `collect` 要做完本轮 mark/sweep 才返回。现在让 VM 在回收步骤之间继续
执行 Lua，就会依次遇到三个问题：**怎样交还控制权，怎样记住进度，怎样修复扫描之后
新增的边**。本章先讲透这条强引用问题链。快照已有的自动 checkpoint、weak table 和
finalizer 保留在后面的扩展阅读，不作为这三个练习的前置知识。

本轮配套代码有两个层次：`exercises/model.rs` 是只含强引用的可运行小模型，用来替换
错误设计、观察确定性轨迹；真实快照仍在 `crates/` 中，另用真实 `Heap` 对照验收。
小模型不是三份历史 VM 快照，也不支持 weak/finalizer。它复用真实 core 的
`GcPhase`、`GcStep` 和 `GcStats`，让概念可以逐项对应，而不混淆验证范围。

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
6. 最后运行 `budget=1`/完整回收对照与 late child 保活验收。

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

## 扩展阅读：快照中已有的自动调度、弱表与 Finalizer

以下内容帮助定位最终 v0.14 已有能力。第一遍可以先完成章末的三个练习，再回来阅读；
本次问题链没有新增 weak/ephemeron/finalizer 的实现或练习。

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

VM 在执行任一 callback 前，把整批 callback 和 userdata 放入 temporary roots，因此一个 finalizer 内部重入 `gc_collect` 不会让尚未执行的兄弟事件提前 second-death。某个 callback 报错时仍执行队列剩余项，随后把第一个错误交还脚本；下一次执行可以继续完成 Sweep，不会留下半周期状态。

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
3 个 VM 测试。本轮新增 8 项强引用问题链参考验收，默认 workspace 合计 **92 项通过、
3 项学习者 TODO 显式 ignored**，分别按下面的入口统计。默认通过只能说明参考合同与原有回归通过，
不表示三个 TODO 已经完成。旧快照里的 weak/finalizer 测试也不等于本轮新增了相关微课。

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

<a id="learning-checkpoint"></a>
## 完成检查点

- 能从“首次做太多”推导出 collect 驱动与 step 的分工；
- 能指出每个返回点之后，phase、gray、sweep IDs、cursor 分别由谁保存；
- 能用八次 trace 解释为什么每次重建进度无法收敛；
- 能先扫描 A、再写入 B，复现并修复遗漏 barrier 的误回收；
- 能区分概念颜色、真实 marked bool、待处理队列，以及 step 的完成信号；
- 三项学习者验收显式运行通过，并能解释模型与真实 Heap 对照的范围。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 collector phase、gray list、barrier 与 weak/finalizer 队列是本版状态机的性能化目标；
- [Table、所有权与共享对象](../ch02-runtime/table.md) 可对照 weak edges 为什么不能用 Rust 所有权直接表达；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可定位安全 checkpoint 为什么放在指令写回之后；
- 下一 checkpoint 是 v0.15：让 coroutine 拥有独立栈与调用帧，并验证 yield/resume 跨越 GC phase 时的 owner turn。
