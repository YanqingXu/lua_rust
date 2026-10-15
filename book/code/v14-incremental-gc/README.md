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

短借用参考 2 项、回调参考 9 项。ignored 不是读者已完成练习；
显式启用时对应四题初始均失败。短借用反例只在 rustc 子进程中编译，保留失败输入不会
破坏默认构建。两个 CLI 固定重放错误/修复，不读取 starter，进程退出 0 只表示重放完成。

## 最后追问：弱边、ephemeron 固定点与 resurrection 顺序

[弱表包](exercises/weak/README.md) 先修普通表扫描造成的弱目标泄漏，再用跨两张弱键
表的链，展示单轮传播遗漏第二个 value。两项 TODO 分别修改边分类与固定点循环；
合同还检查无根环、primitive/metatable、1/4/16 轮链。真实 Heap 对照检查对象实际访问、
弱项清除，以及完整周期之间共享 metatable 的模式刷新。

[复活顺序包](exercises/resurrection/README.md) 用第三项 TODO 交换“pending 强图
补标记”和“weak cleanup”次序。模型、真实 Heap 可控 roots、同一真实 VM 的三次
execute 分别证明弱项可见、rescue 保留和撤根后的 second death；没有重新注册 finalizer。
真实 Atomic 在 finalizer 标记后不重跑 ephemeron，这组弱值测试不覆盖所有混合图。

```powershell
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example weak_problem_chain
cargo run --quiet --manifest-path book/code/v14-incremental-gc/Cargo.toml --example resurrection_problem_chain
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_reference --test gc_resurrection_reference

# 初始分别 2 项、1 项失败；只修对应 starter 后应全部通过。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_weak_exercises -- --ignored
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_resurrection_exercises -- --ignored
```

默认 workspace 合计 **119 项通过、10 项 TODO ignored**：84 项原测试、8 项强引用参考、
11 项借用/回调参考、16 项弱表/复活参考。四个 CLI 都固定重放；学习者完成状态以各
exercise target 的显式 `--ignored` 验收为准。
