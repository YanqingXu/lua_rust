# v0.15：暂停执行数据由谁拥有，GC 又从哪里找得到？

先预测两个互相独立的问题：

1. VM 把 `SuspendedFrame` 保存在 Rust map 中，为什么协程 handle 仍可达，恢复时却会
   遇到失效的寄存器、函数或 vararg？
2. 为了防止失效，把整个 scheduler 和所有 open upvalue 都作为直接 roots，会发生什么？

本包只在**已经完成同步的稳定 suspended 边界**收集。它不模拟字节码、yield 逐层展开、
增量 Atomic/barrier、活动 varargs 或 Lua/宿主错误恢复，不把模型通过当作这些路径已正确。

## 两种所有权必须分开看

真实 VM 的 `CoroutineState.frames` 拥有暂停的执行数据：`function`、`pc`、恢复模式、
`Vec<Rc<RefCell<Value>>>` 寄存器 slots 与 varargs。Heap coroutine 对象保存的是
**克隆出的 `Vec<Value>` 追踪镜像**；执行 slots 没有搬入 Heap，镜像也没有持有 slot 的
引用。在真实源码中，slot 上的借用冲突会变成错误；本模型假定稳定边界没有冲突。

```text
外部强 root → thread → frame.function / register 的 closure → upvalue → slot 当前值
                     → register 当前值
                     → suspended varargs
尚未首次 resume：thread → entry
```

Heap 字段名虽然叫 `Coroutine.roots`，其中的 Value 是**从可达 thread 出发的强边**，
并非把所有 thread 和保存值提升为全局直接 roots。scheduler map 本身也不是全局 root。
`open_upvalues` 的 owner 是 **frame ID**；只有 owner 仍在 `active_frames` 中时，才直接
作为 upvalue root。暂停 getter 的 upvalue 应经 thread 的镜像图到达。

真实 Yield 路径先保存 frame、移除 active 标记并逐层展开，到 `resume_coroutine`
安装暂停 frames 后才同步镜像；这段间隙没有 VM checkpoint/Heap 回收。本题要求的是
**下次允许回收前同步完成**，没有要求源码必须“先 sync，再 detach”。resume 期间旧镜像
暂留，最新运行值由 active slots 保活，旧镜像可能保守地让一些值多活；本模型不复现该阶段。

## 先重放固定反例

从仓库根目录运行：

```powershell
cargo run --manifest-path book/code/v15-coroutine/Cargo.toml --example coroutine_roots_problem_chain -- mirror
cargo run --manifest-path book/code/v15-coroutine/Cargo.toml --example coroutine_roots_problem_chain -- release
cargo run --manifest-path book/code/v15-coroutine/Cargo.toml --example coroutine_roots_problem_chain -- real
```

省略参数或使用 `all` 可重放全部。CLI 使用固定 `faults.rs` 与 `solution.rs`，不读取
starter；退出码 0 只表示固定对照跑完，不表示学习者完成了 TODO。

模型的对象编号是：prefix=1、thread=2、frame function=4、getter open upvalue=5、
cycle=6、register table=7、vararg table=8、getter closure=9；活动 upvalue/value 为
10→11，frame function 的独立 closed capture 为 12→13。首次尚未启动的 entry 使用
独立 fixture 的 3→14，不能把 entry 与暂停 frames 当作必须同时存在的状态。

| 固定输入 | 错误行为 | 修复后必须看到 |
| --- | --- | --- |
| 可达 thread，暂停镜像只保 entry | mirror=[]，Rust state 仍在，恢复检查 stale | mirror=[4,8,7,9]，九对象暂停图可访问 |
| 撤去唯一外部 thread root，scheduler/全部 upvalues 仍直 root | thread 图全存活，states=1 | 暂停图全回收，states=0；活动 10→11 与 prefix 仍存活 |
| 仅去掉 scheduler 直根，仍直 root 所有 upvalues | 5→6→thread→getter 构成自举保活环 | 必须同时按 active frame 过滤 upvalues |

`mirror=[]` 反例说明 Rust 持有执行 frame 与 Rc slots 不等于 GC 会扫描它们。反向的
泄漏则说明“宿主还能索引到这个状态”不能被无条件翻译成“Lua 对象图中仍然可达”。

## TODO 1：把暂停执行值同步成 thread 追踪边

只修改 `starter.rs::mirror_roots`。保留初始 entry，按顺序追加每个暂停 frame 的
function、varargs 和每个 slot 的**当前值**。不要迁移/清空 VM slots，不要把全部保存值
绕过 thread 直接加入全局 roots，也不要只保存函数而遗漏 varargs 或寄存器。

```powershell
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test gc_coroutine_roots_exercises suspended_values_are_mirrored_before_collection -- --ignored --exact
```

初始执行 1 项并失败，恢复检查返回 `stale saved function/register/vararg/capture`。
完成后执行 1 项并通过：暂停图、活动 upvalue 图都活着，fresh entry 图也可达。
模型另验证镜像是快照：修改 Rc slot 不会自动更新旧 Vec，重新同步才更新；slot 的 Rc
身份始终不变。这里复制的是 handle 数值，真实实现复制 Value，不复制其目标对象。

## TODO 2：筛选直接 roots，而非无条件保活所有保存状态

只修改 `starter.rs::extra_direct_roots`。删除 scheduler 的直接 roots，并只保留 owner
仍在 `active_frames` 中的 open upvalues。该 hook 只是本模型的额外根，长期 prefix、
可选外部 thread handle 由 driver 提供；它不是要替换真实整个 `Vm::gc_roots`。

```powershell
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test gc_coroutine_roots_exercises unreachable_continuations_are_not_permanent_roots -- --ignored --exact
```

初始执行 1 项并失败。完成后，有外部 handle 时仍可恢复；移除 handle 后，thread 与
九对象暂停图都回收，VM state 和对应 owner 的 open-upvalue 索引被 prune，活动帧的
10→11 继续存活。仅清 map 或仅检查 state 数量为零都不够，目标对象也必须回收。

两题通过同一 `contracts.rs` 验收，其他 hook 使用参考实现，因此可以独立修复。不要
改合同或默认 ignored 标记。需要答案时再看 `solution.rs`。

```powershell
# 初始 2 项失败；完成 TODO 后应有 2 项通过。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test gc_coroutine_roots_exercises -- --ignored

# 九项默认参考、固定故障与真实 Heap/VM 对照。
cargo test --manifest-path book/code/v15-coroutine/Cargo.toml --test gc_coroutine_roots_reference
```

默认 workspace 显示两题 ignored，不表示练习完成。

## 真实 Heap 与 VM 证明了什么？

真实 Heap 用 public API 构图，对比空镜像与 `coroutine_set_roots` 镜像，属于可控 roots
实验，**没有向真实 VM 注入漏同步故障**。它在 GC 前后都保留宿主 Rust Rc slots，再分别
调用 `function_proto`、`upvalue_get` 与 table 内容访问。输出 graph 八项依次为：frame
function、getter、open upvalue、cycle table、closed upvalue、captured table、register
table、vararg table；不以 object ID 或句柄相等冒充目标存活。

空镜像时 thread 本身可达，八项图访问全失败；正确镜像时全成功。撤去 thread root 后
明确要求 `is_live(thread)==Ok(false)`、八项访问全失败，prefix 内容仍为 42。Rc slot
仍可读取一个 table handle 不妨碍该 handle 的目标已经回收。

真实 VM 分两类实例验证：

- 保留全局 co，在尚未首次 resume 时先 host GC，证明 entry 图保活；带 table vararg
  resume 到 yield 后，结束主执行帧再 host GC，最后恢复计算得到 46。计算同时读取
  closed capture、getter 的 open capture、暂停 register 与暂停 varargs。
- 暂停 getter→cycle.thread→thread 环，只把 cycle/getter/thread 发布到全局弱值表。
  下一次 execute 清 co，结束该观察帧后再 host GC；要求 heap 对象数下降、state_count=0，
  随后新 execute 看到三个弱槽精确 nil、prefix 内容仍为 7。getter 没有 yield 给外层，
  宿主也没有持有返回的对象值。

这些输入都只把布尔值交还宿主，避免上一轮活动寄存器成为隐藏 root。真实活动 varargs
并没有作为独立种类播种到 `gc_roots`；存在 tail-Lua 调用后活动 varargs 在显式 GC 下
失活的既有边界，本题没有修复它。暂停 varargs 的成功不能外推到所有活动调用路径。

永久复现入口是本 README 与 `model.rs`、`contracts.rs`、`real.rs`、`reference.rs`。
实际执行 slots 的 owner、Heap 的追踪边以及完整周期结束后的 state prune 必须分别解释。
