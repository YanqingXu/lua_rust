# Trace 记录的是当前指令执行前，还是执行后？

前置：能读 LoadK/Add/Return 和寄存器编号。约 20 分钟，完成一个 dispatch TODO，
让每条 event 保存当前 opcode 真正的输入状态，并解释复制 Value 与保存活动槽别名
的区别。本包不改生产 VM，不实现完整 VM、深度 Heap 快照或执行回放系统。

## 先预测，再运行

固定手写 Proto 有两个寄存器，初始都为 Nil：

```text
pc 0: LoadK R0, 40
pc 1: LoadK R1, 2
pc 2: Add   R0, R0, R1
pc 3: Return R0
```

预测 pc=2 那条 Add event 中 R0 是 40 还是 42。再预测：若历史保存的是
`Vec<StackSlot>`，克隆 Vec 后，旧 event 还能否保留最初的 Nil？

从仓库根运行。真实 binary 对照需要先构建整个快照 workspace 的 binaries：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example bytecode_trace_problem_chain -- timing
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example bytecode_trace_problem_chain -- vm
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example bytecode_trace_problem_chain -- cli
```

`all`（默认）重放全部对照。CLI 固定使用 faults 和 solution，**不运行 starter**；
即使完成 TODO，before 仍展示相同人为故障，不冒充历史中间 VM。

`timing` 的完整固定输出：

```text
before: record after executing
  step=0 pc=0 line=10 depth=0 op=LoadK registers=[Number(40.0), Nil]
  step=1 pc=1 line=11 depth=0 op=LoadK registers=[Number(40.0), Number(2.0)]
  step=2 pc=2 line=12 depth=0 op=Add registers=[Number(42.0), Number(2.0)]
  step=3 pc=3 line=13 depth=0 op=Return registers=[Number(42.0), Number(2.0)]
after: clone values before executing
  step=0 pc=0 line=10 depth=0 op=LoadK registers=[Nil, Nil]
  step=1 pc=1 line=11 depth=0 op=LoadK registers=[Number(40.0), Nil]
  step=2 pc=2 line=12 depth=0 op=Add registers=[Number(40.0), Number(2.0)]
  step=3 pc=3 line=13 depth=0 op=Return registers=[Number(42.0), Number(2.0)]
before: aliased slot history=[[Number(42.0), Number(2.0)], [Number(42.0), Number(2.0)], [Number(42.0), Number(2.0)], [Number(42.0), Number(2.0)]]
```

after 中，LoadK 的目的值在下一条 event 才出现；Add 的输入是 40 和 2，Return
event 才看到 Add 写回的 42。错误的 after-record 把当前指令的结果提前写进了输入
状态。别名故障则更严重：每条历史都指向同一批活动 Rc 槽，最后全变成 `[42,2]`。

## 一道独立 TODO

只改 [starter.rs](starter.rs) 的
`dispatch(&mut Machine, Instruction) -> Result<(), String>`，先保存再执行：

```diff
-machine.execute(instruction)?;
-machine.record(instruction)
+machine.record(instruction)?;
+machine.execute(instruction)
```

[model.rs](model.rs) 使用真实 `Instruction`、`Opcode`、`StackSlot` 和
`VmTraceEvent`，只处理固定四条线性指令；Return 在这个极小实验中仅作末条标记，
不实现返回窗口、跳转、调用帧或 GC。真实 VM 另执行相同 Proto，独立检查返回 42。
模型只接受已知 fixture，不能把它的索引访问当成任意 bytecode 的校验器。

`record` 读取每个 `Rc<RefCell<Value>>` 中的 Value 并 clone 到独立 Vec。
clone StackSlot 只增加 Rc 引用计数，仍共享活动槽；clone Value 则保存当前寄存器值。
这能冻结 number/nil 等值，却不深拷贝 Value 内的 TableRef/FunctionRef 等 Heap 身份。

共享 [contracts.rs](contracts.rs) 逐项核对四条的 pc、line、depth、opcode 和真实
寄存器值；[solution.rs](solution.rs) 使用同一合同。初始 starter 在 pc=0 失败：
实际 `[40,Nil]`，期望 `[Nil,Nil]`。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_trace_exercises -- --ignored --exact records_current_registers_before_dispatch
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_trace_reference
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_trace_reference failed_instruction_is_recorded_but_decode_and_budget_fail_before_recording -- --exact
```

默认 reference 有 **9 项**：共享合同、after/alias 两故障、真实 VM 与 JSON 元数据、
分支、开关生命周期、失败边界、对象身份/GC root、真实 binary 输出。CLI 或 reference
成功不表示 learner 完成；以 ignored 验收为准。

## 真实 VM 的记录顺序与元数据

生产 `run_frame_state` 每轮先检查预算并减一，再 decode opcode；启用 trace 时读取
并 clone 寄存器值、push event，随后才 dispatch 当前 opcode。真实顶层
`frame_depth = call_stack.len()` 为 **0**，不是 1；它不是源码块的嵌套层数。
本包不把顶层 fixture 扩写成所有 coroutine/tail-call 的深度证明。

手写 Proto 的行号是 10、11、12、13，函数名是 trace-before。`VmTraceEvent`
自身没有 step 字段；`trace_json` 每次序列化时 enumerate，给当前数组从 0 编号。
step 不等于 pc、源代码行或跨所有执行的全局计数。分支对照中 pc=2 被跳过，
四条 event 的 pc 是 `[0,1,3,4]`，JSON step 仍为 `[0,1,2,3]`。

`vm` 的完整固定输出：

```text
real Vm: result=[Number(42.0)] pcs=[0, 1, 2, 3] depths=[0, 0, 0, 0]
real branch: result=[Number(42.0)] pcs=[0, 1, 3, 4] event_count=4
real controls: default/accumulate/take-off/enable-clear/moved=[0, 8, 0, 4, 0]
real failure events: LoadK=1 invalid=0 budget2=2 budget0=0 failed_budget_pc=2
real identity: IdentityReport { earlier_handle_found: true, current_field: Number(42.0), internal_trace_contains_reclaimed_handle: true, republish_rejected_as_stale: true }
```

开关计数依次表示：默认关闭时 0；enable 后连续两次 execute 累积 8；take 关闭记录，
再 execute 为 0；再次 enable 会清旧历史，因此一段旧执行被清掉后只剩新执行 4；
take 转移数组后再次 take 为 0。execute 不会自动清 trace 历史或重新 enable。

失败边界用真实 VM 验证：合法 LoadK opcode 的常量索引越界，dispatch 前已有一条
`[Nil,Nil]` event；invalid opcode 63 在 decode 时就失败，没有 event；预算为 2
时仅记录 pc 0/1，到 pc=2 时预算检查失败，没有 Add event；预算为 0 则一条也没有。
源码中的寄存器快照 `try_borrow` 若失败，也会在 push event 前返回；本包没有注入
私有槽的借用冲突。因此不能笼统声称每个错误都一定有对应 event。

## 保存 Value 既不是保存整个 Heap，也不是给 GC 加 root

真实对象探针先创建 table，把 answer 从 40 改到 42，从较早的 trace 中取得相同
Table 身份，再经 registry 实际访问它的**当前**字段，读到 42。JSON 对对象只输出
type/display 身份，没有保存当时的 table 内容。

另一个 transient table 先产生 trace，再撤掉 global 并执行只返回 bool 的新 chunk，
清除前一次结果 roots。此时旧 Value 仍留在 **Vm 内部 trace_events**；host GC 后，
向 registry 再写该旧值精确返回 `HeapError::StaleHandle`，take 出的 trace 却仍包含
该 typed identity。这个实访问失败与 `gc_roots` 未枚举 trace_events 相互印证。

不要把 Rust 的 Vec/Value 所有权当作 Heap root，也不要仅凭身份文本存在推断对象
可访问。本包展示当前边界，没有修 GC roots，不声称历史记录可安全解引用、可完整
回放或能证明所有 GC/错误路径安全。

## library 的部分 trace 与 CLI 的 stdout 是两份合同

真实 `lua_bytecode --format trace -` 会执行输入。纯 fixture `return 40+2` 被编译
为 **5 条/3 寄存器**：LoadK、LoadK、Add、Move、Return。编译器额外 Move 到返回
窗口，所以不能把手写四条 Proto 的预期直接套给 binary。`real.rs` 保存独立字面
JSON 期望，逐字节核对 binary 的完整成功输出、空 stderr 和 code=0。

runtime fixture `local x=40; error('trace-stop')`：library 返回 Err 后仍可
take 得到 7 项前缀，最后一项是失败 Call 的执行前 event；CLI 在 execute 的 `?`
处返回 code=1，没有走到 take/render，因此 stdout 为空，stderr 含 trace-stop。
不把 CLI 无 JSON 输出误解为 VM 从未记录过。

再运行 `print('script-output');return 42`：Lua print 先把 `script-output\n`
写到同一 stdout，后面才追加 JSON。整个 stdout 已经不是单个 JSON 文档。
因此“纯 trace JSON”演示必须选择无 stdout 副作用的 fixture，不能泛化到任意脚本。

`cli` 的完整固定输出：

```text
real binary: BinaryReport { pure_events: 5, runtime_exit: 1, runtime_stdout_bytes: 0, partial_library_events: 7, print_pollutes_stdout: true }
```

也可单独查看真实 VM 的四条手写 Proto JSON；这个 example mode 不使用源代码编译：

```powershell
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example bytecode_trace_problem_chain -- trace-json
cargo build --release --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --quiet --release --manifest-path book/code/v18-cli-tools/Cargo.toml --example bytecode_trace_problem_chain -- all
```

集成测试从 `env!("CARGO_BIN_EXE_lua_app")` 定位同 profile 的 lua_bytecode；example
也只定位 sibling，不嵌套 cargo、不偷换 PATH 上的工具。共享 process runner 对这些
有界且不启动后代的 fixture 负责 stdin EOF、并行 drain 和 deadline。

理解题：为什么 Add event 的目的寄存器仍为 40，而下一条 Return event 中为 42？
调试题：JSON 中仍有 Table 身份，为什么重新放入 registry 会报 StaleHandle？
完成检查：独立完成 dispatch；解释 before/after/alias 三种轨迹；区分 pc/line/step/
depth、library/CLI、Value snapshot/Heap snapshot/root。迁移到生产只保留 dispatch
前的值快照，固定错误模型和观察 helper 留在教学包。
