# v0.14：finalizer 事件所有权与回调重入

本实验接续上一级的分步 GC 问题链，只讨论**已经从 Atomic 生成的一批事件怎样安全交给
VM 执行**。不修改真实 GC，不加入 weak/ephemeron 固定点或完整的宿主回调系统。

先预测三个问题，再运行同一输入的错误/修复对照：

1. 只 `clone` 事件，回调又调用 GC 时，共享队列里还看得见哪些事件？
2. 用 `mem::take` 拿到独立 `Vec` 后，尚未执行的兄弟事件就一定保活了吗？
3. 第一个回调返回 `Err` 时直接 `?`，后续事件和临时 roots 会怎样？

## 文件与边界

| 文件 | 用途 |
| --- | --- |
| `model.rs` | 只有 handle、显式 roots、userdata→callback 边和一个有界重入点的小模型 |
| `starter.rs` | 三个待修复函数；每题只修改对应函数 |
| `solution.rs` | 可选参考解；合同的其余两个 hook 也从这里取得，避免前题错误掩盖本题 |
| `contracts.rs` | 学习者与参考解共享的行为验收 |
| `faults.rs` | 固定错误版本，不随 starter 修改而变化 |
| `acceptance.rs` | 三道默认 ignored 的 TODO 验收 |
| `reference.rs` | 默认执行的九项参考、故障有效性和真实 Heap/VM 对照 |
| `real.rs` | 真实 v0.14 Heap/VM 探针；没有修改其实现 |
| `../../examples/finalizer_problem_chain.rs` | 固定重放 CLI，不调用 starter |

模型的初始事件已经排好序，每个事件包含 userdata 和 Lua callback 的整数 handle。
`Vec<Event>` 由 Rust 拥有，但模型 GC **不扫描 Rust 栈上的任意 Vec**；只有传入的
roots 和对象强边决定可达性。这与真实 v0.14 的 handle 模型一致，不能外推到所有 GC。

模型只有一次共享的重入额度。错误 clone 仍会显示 `1,1,2,2`，不会真的无限递归。
调用中的当前事件由活动调用帧保活，尚未调用的兄弟事件只能靠整批临时 roots。
roots 中的 `900` 表示进入这段代码前外层已经放好的 root；正确收尾必须恢复到该长度，
不能 `clear()` 把外层 root 一起删除。

## 先重放，再修改

从仓库根目录运行：

```powershell
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example finalizer_problem_chain -- ownership
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example finalizer_problem_chain -- roots
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example finalizer_problem_chain -- errors
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml --example finalizer_problem_chain -- real
```

省略参数或使用 `all` 可一次重放全部。关键结果如下；其余输出给出 take、pin、call、
nested GC 和回收后的状态：

| 输入 | 固定错误版本 | 修复版 |
| --- | --- | --- |
| 两事件，第一个重入一次 | `calls=[1, 1, 2, 2] pending=2` | `calls=[1, 2] pending=0` |
| 同一两事件，只保活当前帧 | `calls=[1] result=Err(Stale(2))`；兄弟两个对象被收 | `calls=[1, 2] result=Ok(())` |
| 三事件，前两个分别出错 | `calls=[1]`；roots 长度从 1 泄漏为 7 | `calls=[1, 2, 3]`；roots 恢复 `[900]`，返回首个 `Callback(1)` |

最后一组还会在结束后再跑 GC：错误版的兄弟对象仍因泄漏 roots 而存活，修复版可以回收。
正常完成的两事件也会在移除整批临时 roots 后被下一轮回收，整批保护不是永久保活。

**CLI 和 reference 测试成功只说明反例与参考解可重放，不表示你已完成 starter。**

## TODO 1：拿走整批事件

修改 `starter.rs::take_batch`。保留 `&mut Vec<Event>` 接口，将长期共享队列与本次调用的
owned batch 分开；不能只 clone，不能靠增加“已经调用过”的额外全局集合掩盖重复。

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_callbacks_exercises transferred_batch_is_invisible_to_reentry -- --ignored --exact
```

初始 starter 失败，显示调用序列 `[1,1,2,2]`。完成后应只调用 `[1,2]`，共享队列为空，
重入 drain 看见空队列，返回后 roots 仍为 `[900]`。

## TODO 2：让整批事件成为临时 roots

修改 `starter.rs::pin_batch`。调用前为**整批**事件加入 callback 和 userdata 的 roots。
只保护当前 invoke 参数不够；不要把 `Vec<Event>` 的 Rust 所有权误认为 GC root。

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_callbacks_exercises whole_batch_roots_protect_siblings -- --ignored --exact
```

初始 starter 失败，返回 `Stale(2)`。完成后第二个事件的 userdata 和 callback 在重入 GC
期间均存活，两事件各执行一次，批次结束 roots 恢复 `[900]`，下一轮允许两个对象回收。

## TODO 3：完成本批事件后返回首错

修改 `starter.rs::drain_batch`。进入时保存 root 长度；保存第一个 `Err`，但继续尝试本批
余下事件；全部处理后 `truncate(root_base)`，再向调用者返回首错。不要求吞掉错误，
也不要求把回调执行过程中可能新产生的所有未来批次一次执行完。

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_callbacks_exercises callback_error_drains_and_restores_roots -- --ignored --exact
```

初始 starter 只执行事件 1。完成后事件 1、2 都会返回错误，事件 3 仍执行；最终返回
`Callback(1)`，而非事件 2 的错误，roots 严格恢复 `[900]`。这里验证正常 `Result`
返回路径的清理，不把它扩大为任意 panic 展开保证。

## 真实实现对照

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_callbacks_reference
```

默认九项应全部通过。其中三项对照不依赖模型：

- 真实 Heap 生成并 `take_finalizers` 得到两个 owned events 后，再运行完整下一轮 GC。
  只把第一个事件加入 roots，第二个事件的 `userdata_is_finalized` 和 `function_proto`
  都访问失败；全批加入 roots 则两者均可访问。探针结束时 Rust 仍持有 events。
- 真实 VM 的第一个 Lua callback 写入序列 1、调用两次 `gc_collect()`、再写入 2。
  第二个 callback 执行 `setfinalizer(value, nil)`，经真实 Heap 的 userdata 访问校验后
  写入 3。最终序列为 `123`；两次 GC 确保不只是推进之前剩下的 Sweep。
- 真实 VM 的第一个 callback 产生 Type 错误，第二个仍运行并写入全局标志；宿主收到
  Type 后，用同一 VM 的下一次执行读到 `true`。临时 root 长度为私有内部状态，其精确
  恢复由模型合同检查，不把这个全局标志冒充真实 VM 内部 roots 的读数。

`userdataid` 只读取 handle 的逻辑 ID，本实验不使用它证明对象还活着。

全部 TODO 的验收为：

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_callbacks_exercises -- --ignored
```

三个 starter 初始都应失败。默认 workspace 测试会把它们列为 ignored；完成后应显式
启用并全部通过。需要答案时再阅读 `solution.rs`，无需修改模型、合同、真实 Heap/VM
或默认 ignored 标记。
