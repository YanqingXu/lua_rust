# Drop 方法体里，谁先释放文件？

本包对应 v0.17 的文件资源问题链末尾：运行期由 `close` 和完整 GC 周期后的
prune 处理资源；VM 本身结束时，需要一个明确的宿主资源关闭顺序。

前置：已能区分 `files: HashMap<UserdataRef, Option<File>>` 的三个状态：
没有 key、存在 `Some(File)`、存在 `None`。约 15–20 分钟，完成一个 TODO，
产出“先清宿主 files，再关闭 Heap”的实现和可重复观察的轨迹。

## 先预测

一个 map 有三个 userdata key：第一个文件已显式 close，留下 `None`，另两个
仍是 `Some(File)`。三个 userdata 又都由一个 Heap table 强引用。

在 `Drop::drop` 中只调用 `heap.shutdown()`，会发生什么？写下三个时点的预测：

1. `heap.shutdown()` 开始时，还有几个 File owner？
2. 自定义 `Drop` 方法体最后一行，还有几个 File owner？
3. 整个 Rust owner 完成析构后，三个 File 各释放几次？

从仓库根运行：

```powershell
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_drain_problem_chain -- before
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_drain_problem_chain -- after
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_drain_problem_chain -- real
```

也可以用 `all`（默认）一次重放。CLI 固定运行 `faults::heap_only` 和
`solution::shutdown`，**不读取 starter**；修完 TODO 后 before 仍展示相同故障轨迹。

## 读懂轨迹

`before` 的完整固定输出：

```text
before: heap-only Drop body
  fixture.explicit_close entries=3 open=2 drops=[1, 0, 0] heap=4
  drop.body.enter entries=3 open=2 drops=[1, 0, 0] heap=4
  heap.shutdown.begin entries=3 open=2 drops=[1, 0, 0] heap=4
  heap.shutdown.end entries=3 open=2 drops=[1, 0, 0] heap=0
  drop.body.exit entries=3 open=2 drops=[1, 0, 0] heap=0
  after complete Rust drop: drops=[1, 1, 1]; reachable_before=[true, true, true]; access_rejected_after=[true, true, true]
```

`after` 在相同 fixture 上输出：

```text
after: explicit files drain
  fixture.explicit_close entries=3 open=2 drops=[1, 0, 0] heap=4
  drop.body.enter entries=3 open=2 drops=[1, 0, 0] heap=4
  heap.shutdown.begin entries=0 open=0 drops=[1, 1, 1] heap=4
  heap.shutdown.end entries=0 open=0 drops=[1, 1, 1] heap=0
  drop.body.exit entries=0 open=0 drops=[1, 1, 1] heap=0
  after complete Rust drop: drops=[1, 1, 1]; reachable_before=[true, true, true]; access_rejected_after=[true, true, true]
```

Rust 会在自定义 `Drop::drop` 方法体返回后，继续自动析构各字段。故障版本的
两个 File 在这个阶段也会释放，整个 owner 结束后并没有永久泄漏。本题检查的是
**Heap shutdown 时文件已经关闭**的顺序合同。两种版本最终计数都为 `[1,1,1]`，
仅检查最终计数无法区分它们。

真实生产 `Vm` 的 `Drop` 已经正确采用下列顺序；本题只修实验 starter：

```diff
 fn shutdown(owner: &mut Owner) {
+    owner.files.clear();
     owner.shutdown_heap();
 }
```

`clear()` 移除三个 map entry。`Some(TrackedFile)` 会释放所拥有的真实 File，
`None` 中已没有 File，所以显式 close 过的第一个文件仍只释放一次。

## 实验与生产实现的对应边界

`model.rs` 使用真实 `lua_core::Heap`、真实 `std::fs::File` 和 HashMap，不实现
第二套 GC。`TrackedFile` 给真实 File 加了一层观察器：先 `drop(file.take())`，
然后才增加计数；没有 `try_clone`，外部 observer 也不拥有 File。此计数观察 Rust
owner 的释放顺序。fixture 显式 close 用 `take()` 留下真实的 `None` entry。

`Owner` 的 `Drop` 运行所选 hook，记录 body 内快照；随后实际发生自动字段析构。
`model::run` 在 `drop(owner)` 返回后再读取 observer，得到最后一行。各文件的
HashMap 析构次序不固定，输出只显示按文件标签排序的计数数组。

GC 前，fixture 通过 table 的三个字段实读 userdata，再访问
`userdata_metatable`，确认它们可达且可访问。shutdown 后 Heap 的对象数为 0，
同一个 getter 精确返回 `HeapError::Shutdown`；这里不会得到 `StaleHandle`，
因为 Heap 首先检查自己是否已关闭。复制 Rust handle 不延长 Heap 生命周期。

`real.rs::heap_shutdown_discards_pending_events` 另用生产 Heap 建两个带 finalizer
的 userdata：一个作为 root 保持可达，另一个在首死亡周期被置为 finalized、
进入待执行队列。不提前 `take_finalizers`；随后直接 shutdown：两个对象都被清除，
待执行队列为空，读取和新分配都返回 Shutdown。

这个 core 探针没有 Lua callback 执行器。它核对队列被丢弃和强制终止；源码
`Heap::shutdown` 直接清 objects/gray/sweep_ids/ready_finalizers，生产 `Vm::drop`
直接调用它，没有再执行 collect 或 finalizer dispatch。因此 shutdown 不是完整
GC 周期，也不会执行 pending finalizer 或 Lua callback；仍可达的 userdata
也不会被保留。这个结论不外推为 panic、进程退出或所有宿主资源的关闭保证。

`real.rs::vm_drop_releases_global_file` 完全不使用上述 hook，创建真实 `Vm`：
第一段 execute 把 open file 留在 global，只返回 bool；host collect 后，同一 VM
第二段 execute 再用 `io.type`、seek、read 实读 `owned`。文件由真实 global root
保活。离开 VM 作用域前后分别尝试独占重开同一路径。

在 Windows，`OpenOptionsExt::share_mode(0)` 的前一次重开必须得到**错误码 32**
（sharing violation，不能用任意错误代替），VM 完成 Drop 后必须独占重开成功，
并从文件读回 `owned`。当前 Windows 的 `real` 输出为：

```text
real Heap: ShutdownReport { reachable_was_unfinalized: true, pending_was_finalized: true, released: 2, remaining: 0, access_rejected: [true, true], events_after_shutdown: 0, allocation_rejected: true }
real VM: global_open=true exclusive_before_blocked=Some(true) exclusive_after_opened=Some(true) contents=owned
```

其他平台仍运行 Heap/RAII 顺序和同 VM 访问检查，但两个 exclusive 字段为 `None`，
CLI 会说明 OS release evidence unavailable。普通 reopen 或删除文件成功，在这些
平台不能证明前一个 File 已关闭。本包的临时目录删除只负责 fixture 清理，不作为
关闭证据。临时目录唯一创建，递归清理前核对 canonical 父路径和原始目录名；
正常路径显式 `finish` 合并主错误与清理错误，Drop fallback 只记录清理错误，避免
在已有 panic 中再次 panic。

## 一道独立 TODO

只改 [starter.rs](starter.rs) 的 `shutdown(&mut Owner)`，保持先释放 File 再关闭
Heap 的顺序。参考解为 [solution.rs](solution.rs)，验收合同为
[contracts.rs](contracts.rs)。不修改 model、faults、real 或生产 crates。

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_drain_exercises -- --ignored --exact drain_before_heap_shutdown
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_drain_reference
```

初始 starter 的 ignored 验收应失败在 `boundary.entries == 0`，实际为 3。
完成后同一合同通过：shutdown 前 entry/open 都为 0，计数 `[1,1,1]`；Heap
关闭前四对象尚在，关闭后为 0 且实访问被拒绝。默认 reference 共四项，检查共享
混合 map 合同、固定故障的自动字段析构、真实 Heap finalizer 队列、真实 VM OS
release。reference 通过并不表示 starter 已完成；以 ignored 验收为准。

理解题：为什么 healthy 和 fault 的最终计数相同，但 shutdown 边界不同？
调试题：若把计数加在真正 drop File 之前，它还能准确证明哪个时点的资源释放？

完成后，应能从 Rust 成员所有权解释两种版本，并指出 shutdown 强制终止与运行期
GC 的区别。迁移回生产代码只保留 `files.clear(); heap.shutdown();`，观察器、hook
和 fixture 不进入 VM。
