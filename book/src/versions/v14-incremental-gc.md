# v0.14 · 增量 GC、弱表与 Finalizer

<div class="lesson-meta"><span>核心增量：可暂停且可重入的回收状态机</span><span>workspace</span><span>checkpoint v0.14</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把一次完整遍历拆成可暂停状态机</strong>
  <p>前置：v0.13 的 roots/edges、mark/sweep 与 Heap owner。新知识：三色不变量、持久 gray
  queue、write barrier、ephemeron 固定点、finalizer 重入。完成后你应能解释任意 mutation
  怎样保持黑对象不遗漏白 child。</p>
</div>

v0.13 每次 `collect` 都会一次扫描整个对象图。正确性已经成立，但暂停时间随 heap 增长；mutator 若在扫描中修改黑对象，还会破坏“黑对象不指向白对象”的不变量。本版增加一个核心能力：**完整 GC 可以在多个 VM checkpoint 之间暂停和恢复，同时保持弱引用与 finalizer 的可验证语义**。

<a id="learning-predict"></a>
## 先预测再运行

设 parent 已被扫描成 black，child 仍是 white，此时脚本执行 `parent.x=child`。若没有 barrier，
gray queue 已不会重访 parent，child 将被错误 sweep。先画出错误路径，再选择修复：把 child
染 gray，或把 parent 重新染 gray。本版采用前者。

<a id="learning-code-map"></a>
## 代码阅读地图

1. `GcPhase`：Pause/Mark/Atomic/Sweep 的合法迁移；
2. collector state：跨 step 保存 gray stack、sweep IDs、cursor 和统计；
3. `step(budget, roots)`：消费有限 work unit；
4. Heap mutation API：集中执行 insertion barrier；
5. allocation checkpoint：指令写回后按 debt 推进 step；
6. Atomic：ephemeron 固定点、weak 清理、finalizer 发现；
7. VM finalizer drain：释放 Heap 借用后回调，允许重入；
8. budget=1 与 full collect 对照测试：证明两种调度结果一致。

先只掌握强引用和三色不变量，再读 weak/finalizer。后两者不是独立算法，而是在普通 mark
无法直接决定生死时，把决定延迟到 Atomic 一致性边界。

## 四个阶段

```text
Pause ──开始周期──> Mark ──gray 为空──> Atomic ──固定点──> Sweep
  ^                                                        │
  └──────────────────── 周期完成、重置 debt ────────────────┘
```

| Phase | 本阶段工作 | 是否按普通预算暂停 |
|---|---|---|
| `Pause` | 等待 allocation debt 达阈值 | 是 |
| `Mark` | 从 gray stack 每次处理至多 `budget` 个对象 | 是 |
| `Atomic` | ephemeron 固定点、finalizer 发现、弱引用清理 | 否，作为一致性边界完成 |
| `Sweep` | 按 object ID 列表删除 white record | 是 |

`step` 的核心控制流可以先压缩为：

```text
Pause:  seed roots, enter Mark
Mark:   pop at most budget gray objects and trace edges
Atomic: repeat ephemerons to fixed point; queue finalizers; clear weak entries
Sweep:  delete at most budget white snapshot IDs; finish cycle
```

[查看完整 incremental step](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v14-incremental-gc/crates/lua_core/src/lib.rs)

每一步都会重新播种当前 roots，但只把 white root 加入 gray stack。若反复把已经 black 的 root 压到 LIFO 顶部，预算为 1 时真正的 child 会永远饥饿；对应回归测试正是从这个边界产生的。

`collect` 不再维护另一份算法，而是以无限预算反复调用同一个 `step` 直到周期结束。完整与预算为 1 的增量模式在相同对象图上必须得到相同 `GcStats`。

## Write barrier

Mark 期间若已经 black 的 table 写入尚未标记的 child，只依靠旧扫描结果会让 child 在 Sweep 中消失：

```text
black parent ──新写入──> white child   ✗
             barrier ──> gray child    ✓
```

```rust
{{#include ../../code/v14-incremental-gc/crates/lua_core/src/lib.rs:write_barrier}}
```

本版把 barrier 集中在 Heap mutation API：table key/value、metatable、closed/open upvalue 与 userdata finalizer 写入都不能绕过。Mark/Atomic 中新分配对象进入 gray；Sweep 中新对象直接标记为存活，因为它不属于该周期开始时的 sweep snapshot。

Stack 与 global 不经过 Heap 容器写入，所以每个 step 重扫 root handles，等价于 root barrier。测试会先把 parent 标黑，再把预先存在的 white child 写入，随后完成 GC 并验证 child 仍可访问。

## Allocation accounting 与自动 checkpoint

Heap 为每次 Table、Closure、Upvalue、Userdata 分配累计 debt。达到阈值，或一个周期已经处于活动 phase，VM 会在每条指令完成并写回寄存器后推进少量工作。checkpoint 选在写回之后，避免刚创建但尚未进入 root 的对象被误回收。

`with_gc_allocation_threshold(1)` 的压力测试在 100 次循环分配中让每个安全点推进 GC，同时验证寄存器、循环清理和 barrier 不产生 stale handle。周期完成后 debt 归零；v1.0 保留教学用对象计数合同，并在 `LUA51-D010` 中显式登记与官方字节计账/调参接口的差异。

## Weak value、weak key 与 ephemeron

Table 在设置 metatable 时读取 `__mode`：包含 `v` 时不从 table 标记 value，包含 `k` 时不直接标记对象 key。之后修改 metatable 的 `__mode` 会刷新所有依赖 table 的模式。

Weak value 可以在 Atomic 直接依据 mark bit 删除。Weak key 更微妙：若 key 从其他 root 可达，那么它对应的 value 必须保留；value 又可能使其他 weak key 可达，所以需要重复到固定点：

```text
key 已标记 → 标记 value → 传播新 edges → 再检查 ephemeron
```

Atomic 反复执行“已标记 key → 标记 value → 传播 edges”，直到没有新对象被标记，再清理
未标记弱边并生成 finalizer 事件。
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

本快照共有 84 个测试：59 个端到端测试、7 个编译器测试、15 个 core 测试和 3 个 VM 测试。增量新增覆盖完整/单步一致性、显式 step 完成信号、phase 经过、barrier、自动阈值、三种 weak mode、ephemeron、finalizer 顺序、错误 drain、重入、resurrection 与 second death。

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
  <p>构造两层 ephemeron，让第一个 value 使第二个 key 可达；逐轮写出 Atomic 新增的 mark，
  说明为什么必须迭代到固定点。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>把 gray stack 改成 queue，记录 budget=1 的遍历顺序和最大容器大小；要求最终 GcStats
  与 full collect 保持一致。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>删除 table value barrier，复现 black→white 边导致的 stale handle；在恢复 barrier 后
  增加一条只覆盖该 mutation family 的回归测试。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能写出四个 phase 的状态迁移和持久字段；
- 能为每类 Heap mutation 判断是否需要 barrier；
- 能解释 weak key/value 与 ephemeron 的差别；
- 能说明 finalizer 为什么必须在释放 Heap 借用后执行。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 collector phase、gray list、barrier 与 weak/finalizer 队列是本版状态机的性能化目标；
- [Table、所有权与共享对象](../ch02-runtime/table.md) 可对照 weak edges 为什么不能用 Rust 所有权直接表达；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可定位安全 checkpoint 为什么放在指令写回之后；
- 下一 checkpoint 是 v0.15：让 coroutine 拥有独立栈与调用帧，并验证 yield/resume 跨越 GC phase 时的 owner turn。
