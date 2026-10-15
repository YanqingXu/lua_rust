# v0.16：sort 的 Rust snapshot 由谁保活？

**真实 v0.16 当前存在一个可运行的 GC 失活入口，下面的健康 proposal 没有修复核心。**
本题只完善教学实验：理解 Rust `Vec<Value>` 的拥有权、显式 GC roots 的生命周期，以及
callback 返回 Err 时如何恢复已有 roots。默认参考成功与真实已知失败必须分别记录。

## 先运行真实故障，确认不是猜测

从仓库根目录运行：

```powershell
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml -- book/code/v16-core-stdlib/examples/sort_snapshot_roots_gap.lua
```

**当前预期：退出码 2，诊断包含 `stale GC handle`。** 这是独立
`known-functional-failure / functionality_pass=false`，不计为默认通过或 learner ignored。
它也不在 `sort_roots_problem_chain` 默认 all 中运行。

该输入先让 `make_source()` 返回四张 rank 表，构造帧结束后才 sort，避免构造时的临时
寄存器暗中保活四个元素。第一次 comparator 清空源表四个槽，再 `gc_collect()`：

```text
sort 的 Rust Vec<Value> → 四个 table handle（collector 不扫描任意 Rust Vec）
当前 invoke / callback frame roots → 正在比较的两个 table
源表仍是 root，但已被 comparator 清空 → 不能再保活其余两个 table
后续比较读取 victim.rank → 真实 Heap 访问失败
```

真实 `TableFunction::Sort` 先复制数组为局部 values，再做 insertion sort，全部比较
成功后才回写；没有把整个 values 发布为 temporary roots。`invoke` 保护 callable 和
本次两个参数，不能代替完整 snapshot 的 root 生命周期。

这段输入不是官方 Lua 的 oracle 等价证明。本题把“自己选择的 Rust snapshot 算法所
持有的对象，在 callback 返回后仍可安全访问”作为**本 VM 的内部所有权合同**；不宣称
官方 Lua 对 comparator 修改源表时的最终排序结果有保证。proposal 得到 1/2/3/4 是
该独立模型的约定结果，不是新增的 Lua 兼容承诺。

## 用同一交错顺序重放错误与 proposal

```powershell
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example sort_roots_problem_chain -- liveness
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example sort_roots_problem_chain -- errors
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example sort_roots_problem_chain -- real
```

省略参数或使用 `all` 可重放全部。CLI 固定使用 `faults.rs` 与 `solution.rs`，不读取
starter；成功退出只表示教学对照执行完成，不表示 TODO 已完成，更不表示真实缺口已修复。

模型直接使用真实 `Heap`，source 及既有 prefix table 为根，每项 table 的 rank 为
4、3、2、1。callback 由固定操作模拟，不是完整 Lua callable；当前 pair 在每次比较
前放入 roots，比较返回后恢复。模型不覆盖 yield、panic/abort 或所有增量 phase。

| 对照 | 可观察结果 |
| --- | --- |
| 只依赖当前 pair | 第一次 GC 后 rank 4/3 仍可读，rank 2/1 实际访问失败；第二次比较 stale |
| proposal：全 snapshot 临时 roots | 四项持续可访问，完成排序并回写，结果 1/2/3/4 |
| 成功后清空 source，再 GC | 四项 `is_live==Ok(false)`，实际 rank 访问全部失败 |
| 全 snapshot 入根后用 `?` 早退 | root 长度从原有 1 泄漏为 5；被 source 替换的旧对象仍可读 |
| proposal：保存 Result 后清理 | Err 时 roots 恢复精确前缀；被替换的旧对象可回收，prefix 仍为 900 |

`SortVm.items` 仍保存 Rust handle 用于验证失活；它不是 GC root，也不把表对象复制出来。
故障与释放检查都读取真实 table 字段，不只比较对象 ID 或总数。

## 一个 TODO：给整个操作建立 root 作用域

只修改 `starter.rs::with_snapshot`，必要时补充导入。输入是 VM 的可变借用、可变排序
snapshot 和预置 operation。先保存旧 root 长度，加入**所有 snapshot 值**，执行包含
比较与最终回写的 operation，再恢复旧长度并交还原 Result：

```diff
- operation(vm, values)
+ let root_base = vm.temporary_roots.len();
+ vm.temporary_roots.extend_from_slice(values);
+ let result = operation(vm, values);
+ vm.temporary_roots.truncate(root_base);
+ result
```

不能把 `operation(...)` 改成带 `?` 的早退，也不能 `clear()` 丢掉进入时已有的外层
roots。`Vec` 内的顺序会交换，但根只需保持同一整组 Value 可达，不能只 root 首个 pair。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test sort_roots_exercises snapshot_roots_survive_callbacks_and_restore_on_error -- --ignored --exact
```

初始执行 **1 项并失败**，得到 stale Heap handle。修复后同一项通过，且合同继续验证
第二种场景：第二次比较把 `source[1]` 写成 99，随后返回 `Err(Comparator)`。

这时排序的最终整批回写不会执行，但 callback 已写入的 99 **不会回滚**。正确 root
清理只恢复资源生命周期，不撤销 callback 的语言副作用。移除 snapshot roots 后，原
rank 4 对象已不在 source 中，下一次 GC 必须让它失效；prefix table 必须仍可读取 900。
因此不能只检查“返回 Err”或“根计数变小”，还要检查精确前缀和实际释放。

`solution.rs` 是可选答案。不要修改 operation、合同、固定 faults、真实核心或 ignored
标记。默认 workspace 的 1 项 ignored 不表示学习者已完成。

## 默认参考与真实 VM 正向范围

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test sort_roots_reference
```

五项默认参考分别验证 proposal 合同、两个固定模型故障，以及两个真实 VM 正向输入：

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test sort_roots_reference real_vm_sort_with_unchanged_source_and_lua_truthiness -- --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test sort_roots_reference real_vm_comparator_error_does_not_rollback_callback_mutation -- --exact
```

第一项中 source 的元素槽保持不变，comparator 每次 GC 后读取 rank，排序成功；这时
source 本来就保活所有元素，所以不能把它当作清源缺口已经修复的证据。同一输入还在
两元素单次比较中分别返回 false、nil、0：前两个不触发交换，0 在 Lua truthiness 下
触发交换。这只检验返回值解释，不把“永远返回 0”当作任意输入的有效排序关系。

第二项通过 `pcall` 观察 comparator 报错：调用两次、源表第一个槽仍为 99，第二个槽
仍为原来的 3，错误文本含 `sort-fail`。它验证副作用和错误传播，不直接读取真实 VM
私有 temporary roots；精确资源前缀/释放由 proposal 合同与源码阅读核对。

永久入口是本 README、`model.rs`、`contracts.rs`、`real.rs`、`reference.rs` 与上面的
独立已知失败 Lua 文件。真实核心修复须另建工程工作包，不能把教学 proposal 的通过
写成真实 VM 已完成修复，或扩展为全部 callback/GC 路径安全。
