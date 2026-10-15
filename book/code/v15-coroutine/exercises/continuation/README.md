# v0.15：暂停的是哪个 coroutine，恢复时从哪里继续？

本包从三个具体失败出发：共用 scratch 覆盖另一 coroutine 的局部值；恢复时从 pc=0
重跑入口副作用；嵌套 resume 没有转交 owner turn，父子 yield 通道混在一起。
三个 TODO 独立验收，其余 hook 使用健康参考设计，不要求先完成上一题。

文件中 `model.rs` 是固定小程序的独立模型，**不是历史 VM 快照**；`real.rs` 另用当前
真实 v0.15 编译器/VM 执行 Lua。CLI 固定重放 `faults.rs` 和 `solution.rs`，不会调用
学习者 `starter.rs`。默认参考通过与 CLI 退出 0，都不表示已完成 TODO。

## 先预测，再运行

从仓库根目录运行，省略参数或使用 `all` 会重放全部：

```powershell
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml --example continuation_problem_chain -- storage
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml --example continuation_problem_chain -- pc
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml --example continuation_problem_chain -- turn
cargo run --quiet --manifest-path book/code/v15-coroutine/Cargo.toml --example continuation_problem_chain -- real
```

模型的 coroutine 每次只执行固定五条逻辑指令：入口副作用并初始化 local → yield local →
加本次 resume 输入 → 再 yield → 加最后一次输入并 return。一次 resume 至多经过两条
逻辑指令；CLI 的调用次数固定，故障版不会无限运行。`Value::Number/Nil` 是模型值，
不含真实 GC handle。返回的 `Yield`/`Return` 都保留 nil，便于观察多值通道。

## TODO 1：一个 scratch，为什么不能保存两个暂停栈？

co1 的 seed=10，co2 的 seed=100。它们各自第一次 yield 后，保存的 local 分别应该是
10 和 100。随后交错传入 1、2、3、4：

| 调用 | 共用 scratch 的固定错误版 | 每个 co 拥有 frame |
| --- | --- | --- |
| co1 第一次 | Yield 10,nil | Yield 10,nil |
| co2 第一次 | Yield 100,nil，覆盖 scratch | Yield 100,nil |
| co1 输入 1 | Yield **101**,nil，读到 co2 的 local | Yield 11,nil |
| co2 输入 2 | 提前 Return **103**,nil | Yield 102,nil |
| co1 输入 3 | LostFrame(1) | Return 14,nil |
| co2 输入 4 | Dead(2) | Return 106,nil |

只修改 `starter.rs::save_frame`。输入 frame 已拥有 local 与**yield 后的下一条 pc**，
将它交给对应的 `Coroutine.frame`，不要丢进全局 scratch：

```diff
- *scratch = Some(frame);
+ coroutine.frame = Some(frame);
```

模型特意保留“私有 frame 缺失时读 scratch”的故障后备入口，让错误设计能运行到污染
而非立刻 panic。健康实现从不使用 scratch，合同要求最后 scratch 仍为空；这条后备
逻辑不是建议真实 VM 保留的 API。

```powershell
# 初始执行 1 项且失败；完成 TODO 1 后通过。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_exercises suspended_frames_belong_to_each_coroutine -- --ignored --exact
```

验收还检查两个独立保存的 local、交错返回结果、入口副作用各一次及最终 Dead。
把两个 seed 改成相同值、改断言或停止交错，都无法证明所有权正确。

## TODO 2：恢复为什么不能再次从函数入口执行？

只运行一个 seed=10 的 co，消除栈污染的干扰。第一次 yield 10,nil 后，resume 输入 5
应该在保存的 pc=2 接着执行，得到第二次 yield 15,nil；再输入 7，应 return 22,nil。
错误 `resume_pc=0` 每次都重新运行入口，三次都 yield 10,nil，入口副作用从 1 变成 3。

只修改 `starter.rs::resume_pc`，沿用第一题已经独立保留的 frame：

```diff
- 0
+ frame.pc
```

```powershell
# 初始执行 1 项且失败；完成 TODO 2 后通过。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_exercises resume_does_not_restart_entry_side_effects -- --ignored --exact
```

这不是仅仅改变一个整数。真实 `SuspendedFrame` 保存 function、registers、varargs、
下一条 pc、frame_id、resume_mode；恢复的 resume 参数要先写入原 yield 调用的结果
窗口，然后执行下一条指令。模型只用一个整数 input，不模拟寄存器结果窗口或任意调用栈。

## TODO 3：子 co 运行期间，父 co 的 owner turn 在哪里？

父 co 让出控制权给子 co 时，父状态应临时为 Normal，子状态为 Running；子 yield 后，
父恢复 Running，子变 Suspended。yield payload 属于这次子调用，不能留在父通道里。

`TurnVm` 使用**人工非空哨兵**：父 values=[900]、frames=[outer-frame]；子产生
values=[7,nil,9]、frames=[child-frame]。这些哨兵专门检查隔离与恢复，不能声称普通 Lua
嵌套 resume 输入一定产生同样的非空外层 yield 通道。真实 Lua probe 另检查可见状态和
真实多值方向。

错误版在子体中观察到父 Running、非空外层通道，输出混为 [900,7,nil,9]，帧也混合。
修复后应观察父 Normal、空子通道，只向调用者交付 [7,nil,9] 和 child-frame；父原有
[900]/outer-frame 原样恢复。

只修改 `starter.rs::run_child`；可选参考是 `solution.rs` 的同名函数：

```diff
+ let parent = vm.current;
+ vm.statuses[parent] = Status::Normal;
+ let outer_values = vm.yield_values.take();
+ let outer_frames = std::mem::take(&mut vm.yield_frames);
  // 切换 current/status，执行子体，取出子 values/frames。
+ vm.current = parent;
+ vm.statuses[parent] = Status::Running;
+ vm.yield_values = outer_values;
+ vm.yield_frames = outer_frames;
```

`take` 转移 Vec 的所有权，让共享槽为空；它不是深复制栈、GC rooting 或并行线程切换。
本模型只有一个有界嵌套层，不含错误/panic 清理；不能由它推断任意 native 重入安全。

```powershell
# 初始执行 1 项且失败；完成 TODO 3 后通过。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_exercises nested_owner_turn_restores_parent_and_channels -- --ignored --exact
```

## 默认验收与真实 VM 对照

```powershell
# 9 项通过：3 共享正确合同、3 固定故障、3 真实 VM 对照。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_reference

# 初始三题均失败；完成后均通过。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test co_continuation_exercises -- --ignored
```

默认 workspace 把三道 TODO 标为 ignored。显式验收无需删掉 `#[ignore]`，也不要修改
模型、faults、contracts 或真实 VM 来配合答案。[real.rs](real.rs) 的三组 probe 为：

- 同一次 Lua 执行中交错两个 co，结果为 10/100、11/102、14/106，两个 co 最终 Dead；
- 同一个真实 VM 分三次 host execute：先在 inner Lua 调用中 yield，恢复 inner 后又在
  caller 中 yield，再 return。精确多值分别为 `(true,10,nil)`、`(true,21,nil)`、
  `(true,32,1,nil)`；1 是入口副作用累计次数，`execute_multi` 的 Vec 保留末尾 nil；
- inner 检查 parent 为 Normal、自己为 Running，outer 在每次返回后为 Running；
  resume 输入 `(7,nil,9)` 成为 inner yield 的返回值，再随 return 回到 outer。
  最后再读两者 Dead、主线程 `coroutine.running()==nil`。

第二组确实跨越一层 Lua caller 的 SuspendedFrame，而不是只重新调用协程入口。第三组
的 `resume` 成功布尔值属于调用者看到的结果，不能当成子 coroutine 内 yield 表达式
收到的第一个参数；中间的 nil 也不能因“没有值”而删除。

真实源码入口见 [lua_vm/src/lib.rs](../../crates/lua_vm/src/lib.rs)：Call/TailCall 收到
Yield 时保存 `pc=next_pc`；`resume_frame` 根据 resume_mode 写回结果；`resume_coroutine`
从目标 state `take` frames，切换父 Running→Normal，保存外层通道，运行子 co，再恢复
父 owner 和通道。真实 registers 是 `Vec<Rc<RefCell<Value>>>`；`registers.clone()`
复制 Rc 别名，并非深复制 Value。独立 coroutine 拥有自己的 continuation 和槽组，
同一帧的不同观察入口仍可能共享同一 StackSlot。

本包不验证 GC roots、关闭 Upvalue、错误后全部清理、所有 tail/native/metamethod
边界、任意调用深度或并发。这些边界须读既有真实测试或后续专门练习，不从三条
简化问题链外推。原快照源码和既有测试均未修改。

理解题：为什么只保存 status 不足以恢复执行？为什么 frame 的 next pc 不能重置？
实现题：分别完成三个 hook，并运行对应单题命令。
调试题：只在交错时出现 101，应先检查共享 frame owner，还是先修改加法指令？
完成时应能解释 frame/当前执行 owner/yield 通道各自由谁拥有，以及参数向哪个方向流。

本机验证使用 `target/teaching-v15-2026-10-15/` 独立日志目录；忽略产物不随 clone
交付，永久复现入口是本 README 和已提交的源文件。
