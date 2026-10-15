# v0.14：弱值清理为什么要等 pending finalizer 强图标记完？

本实验从一个具体反例出发：userdata U 已不可从普通 roots 到达，但它有尚未执行的
finalizer。weak-value 表 W 同时保存 U、callback F、F 的强子对象 C 和 C 的子对象 D。
如果先清理 weak 条目，再标记 pending 对象，它们当轮仍会存活，四个 weak 条目却
已经永久丢失。callback 后续发布 rescue root 也不能把删除的条目自动补回来。

目标是调整一处顺序，并解释三种生命周期状态：首轮 pending、rescue 后继续存活、移除
rescue root 后 second death。这里验收的是**当前 v0.14 checkpoint 的 weak-value 合同**，
不把它当作官方 Lua 的 finalizer/weak 混合语义证明。

## 先预测：哪个时刻才运行 callback？

真实 core 的 Atomic 顺序为：

```text
已有 ephemeron 固定点
→ 发现 finalizable userdata，设 finalized=true，排队事件
→ 标记 pending userdata/callback 并排空它们产生的强边
→ cleanup_weak_tables
→ 准备 Sweep，Heap 返回
→ VM 取得 owned batch、放入整批 temporary roots，才调用 callback
→ callback 可以发布 rescue 到 global/registry
```

所以保留当轮 weak 条目的理由是 **pending 强图已标记**，不是 callback 赶在清理之前
执行了 rescue。当前源码在 finalizer 标记之后**没有第二轮 ephemeron 固定点**；本题
只使用 weak-value，不能用它外推“finalizer 晚标记 weak key 后，其 value 一定正确补标”。

模型只包含普通强边 `U→F→C→D`、四个 weak-value 槽和强前缀 roots `P/W`。C 在真实
闭包中通过 closed upvalue 到达，真实对照额外访问该 upvalue。模型不实现 ephemeron、
增量 budget、回调重入、重新注册 finalizer、weak key 或完整 VM。

## 先运行固定前后对照

从仓库根目录执行：

```powershell
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example resurrection_problem_chain -- order
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example resurrection_problem_chain -- cycles
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example resurrection_problem_chain -- real
```

省略参数或使用 `all` 可重放全部；CLI 使用 `faults.rs` 和 `solution.rs`，不读取 starter。
成功退出只表示固定对照跑完，不能据此判断学习者完成了 TODO。

数组的四项依次为 userdata、callback、child、grandchild。`pending` 是本轮准备回调前的
队列长度，`calls` 是模型实际执行 callback 的累计次数；`prefix` 同时检查 P 与 W 仍活着。

| 观察点 | weak 四槽 | U/F/C/D 实际存活 | pending | 累计 calls |
| --- | --- | --- | --- | --- |
| 错误：先 cleanup，再 mark pending | 全 false | 全 true | 1 | 1 |
| 修复：先 mark pending 强图，再 cleanup | 全 true | 全 true | 1 | 1 |
| rescue 后第二周期 | 全 true | 全 true | 0 | 1 |
| 移除 rescue 后第三周期 | 全 false | 全 false | 0 | 1 |
| 从未 rescue 的第二周期 | 全 false | 全 false | 0 | 1 |

轨迹会按顺序显示 `Heap: cleanup weak values`、`Heap: prepare Sweep; return to VM`、
`VM: invoke callback`、`VM: publish rescue root`。不要把 callback 箭头画到 weak 清理之前。
强前缀 P/W 在上述每个观察点都保留。

## 唯一 TODO：交换两项操作

只修改 `starter.rs::finish_atomic`，不修改模型、合同或真实 core/VM：

```diff
- heap.cleanup_weak();
  heap.mark_pending_graph();
+ heap.cleanup_weak();
```

`mark_pending_graph` 必须追踪完整强图；只标记 U 而遗漏 F/C/D 也不能满足合同。
不要把 weak 表改为强表、增加 U/F/C/D 的长期 root，或在 callback 中重建 weak 条目。

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_resurrection_exercises pending_graph_is_marked_before_weak_cleanup -- --ignored --exact
```

初始应执行 **1 项且失败**：pending 和对象存活都没问题，但 `weak_before_callback`
实际为 `[false;4]`，合同要求 `[true;4]`。修复后应执行 1 项且通过，并同时验证：

- callback 在 Heap 清理并返回之后才执行，能看到四个 weak 条目；
- 发布 rescue 后下一周期保持强图，且 finalized 仍为 true，不重复排队；
- 移除 rescue 后 weak 槽清除、四对象回收，累计 callback 仍为一次；
- 不发布 rescue 的对照在第二周期就死亡；P/W 始终保留。

默认 workspace 测试将这一题列为 ignored。默认通过不表示你已完成练习；需要答案时
再读 `solution.rs`，显式 TODO 验收无需修改 `#[ignore]`。

## 默认参考与真实实现证据

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_resurrection_reference
```

默认六项都应通过：修复合同、固定早清理反例、模型 no-rescue，以及以下三项真实对照：

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_resurrection_reference real_heap_keeps_the_pending_graph_then_rescues_and_releases_it -- --exact
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_resurrection_reference real_heap_without_rescue_has_second_death_in_the_next_cycle -- --exact
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_resurrection_reference real_vm_callback_sees_weak_entries_and_rescue_lasts_until_released -- --exact
```

真实 Heap 使用显式 roots 驱动完整周期，首轮 `take_finalizers` 后模拟 VM 把 U 加入
长期 root；它不真正执行 Lua callback，所以 `events=1` 只表示累计生成一个事件。
移除 root 后精确要求四个 `table_get(weak,key)==Ok(Nil)`，不能把 weak 容器被误收产生
的 Err 当成“槽已清除”。另外以 `userdata_is_finalized`、`function_proto`、`upvalue_get`
和实际 child/grandchild `table_get` 验证强图可访问或已失效，P 的内容仍为 7。
Rust 局部 Vec/handle 不是这些 API 的 roots。

真实 VM 由同一个 VM 连续执行三个独立输入，分别只返回布尔值：

1. setup 函数返回后才 GC；callback 检查四个 weak 槽、访问 child 的 42，并将 U 发布为
   全局 `rescued`。插入临时强表 `probe[value]=42` 会真实校验 userdata key。
2. 再次 GC 后，rescue 仍保活强图；再次通过 `probe[rescued]=42` 校验 U，读取 child 的
   42 与 prefix 的 7，累计 callback 仍为 1。
3. 新执行开始后移除全局 rescue，再 GC，四槽均 nil、累计 callback 仍为 1、prefix 为 7。

分三次执行使前一次输入的活动帧和临时寄存器在下一次开始前清空，避免观察表达式中的
局部 handle 被误当成额外保活证据。三个返回值应均为 `true`。这里没有使用只读取 ID
的 `userdataid`，也没有再次调用 `setfinalizer` 来测试存活：后者会重置 finalized，改变
本题“不重新注册时只执行一次”的前提。

完整源文件见 `model.rs`、`contracts.rs`、`real.rs` 与 `reference.rs`；可选答案在
`solution.rs`。所有反例都限制在已说明的 checkpoint 边界内。
