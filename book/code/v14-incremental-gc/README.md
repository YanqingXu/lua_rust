# v0.14 · 增量 GC、弱表与 Finalizer

本快照把 v0.13 的 stop-the-world 完整回收拆成 `Pause → Mark → Atomic → Sweep`，每个 VM checkpoint 按预算推进。Table、metatable 和 upvalue 写入统一执行 black-to-white barrier；分配计数达到阈值后自动启动回收。

```powershell
cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml -- `
  book/code/v14-incremental-gc/examples/demo.lua
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --workspace
```

弱表读取 metatable 的 `__mode = "k" | "v" | "kv"`；weak key 使用 ephemeron fixpoint。最小 userdata 可通过教学接口 `newuserdata` / `setfinalizer` 注册 Lua finalizer，支持重入、resurrection 和只执行一次后的 second death。

`gc_step`、`gc_phase`、`gc_collect`、`gc_live` 以及 userdata helper 仍是阶段诊断 API。完整标准库、Lua 5.1 userdata/debug API、coroutine 与最终兼容命名尚未实现。

## 三个问题的可运行对照

[实验包](exercises/README.md) 将分步回收、跨调用保存进度、write barrier 拆成三个
可运行反例。小模型独立于真实 GC；参考验收另以真实 `Heap` 检查相同对象图。

```powershell
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example gc_problem_chain
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_reference
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_exercises -- --ignored
```

参考验收 8 项通过；学习者三个 TODO 初始显式运行失败，修复后应有 3 项通过。
CLI 固定重放前后设计，完成练习以 `gc_exercises -- --ignored` 为准。

## 继续追问：短借用、事件所有权与回调重入

[短借用实验](exercises/borrow/README.md) 从实际 rustc `E0499` 反例开始，提取拥有型
子边后结束 record 借用，再重新借用 Heap 排队。它同时验收编译与扫描结果，避免通过
删掉循环来消除错误。

[事件与回调实验](exercises/callbacks/README.md) 分别替换三个设计：clone 队列改为
`mem::take`，当前调用帧保活改为全批临时 roots，`?` 立即退出改为记录首错、继续 drain、
恢复外层 roots 后返回。独立模型提供固定反例，真实 Heap/VM 另验证对象访问与调用行为。

```powershell
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_borrow_reference --test gc_callbacks_reference
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example finalizer_problem_chain

# 初始分别 1 项、3 项失败；完成对应 starter 后应全部通过。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_borrow_exercises -- --ignored
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_callbacks_exercises -- --ignored
```

短借用参考 2 项、回调参考 9 项。默认 workspace 合计 **103 项通过、7 项 TODO ignored**
（84 项原测试、8 项强引用验收、11 项借用/回调验收）。ignored 不是读者已完成练习；
显式启用时七题初始均失败。短借用反例只在 rustc 子进程中编译，保留失败输入不会
破坏默认构建。两个 CLI 固定重放错误/修复，不读取 starter，进程退出 0 只表示重放完成。
