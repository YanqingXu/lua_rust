# 文件句柄何时跟随 userdata 释放？

这一题只修改 `starter.rs::prune`，完成后能解释两个独立所有者：Heap 管理 Lua
userdata，Rust `HashMap<UserdataRef, Option<File>>` 管理 OS File。map 的 key 和
宿主保存的 handle 都是可复制句柄，不会自动成为 GC roots。

先预测四格 fixture 的结果。数组顺序固定为：可达 open、可达 closed tombstone、
不可达 open、不可达 closed tombstone。closed 使用真实的 `None` map 槽；创建时
原 File 已 drop。userdata 都没有 finalizer，roots 只包括前两个 userdata。

从仓库根目录运行固定回放：

```powershell
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_prune_problem_chain -- all
```

`gap|prune|roots|real|all` 分别观察漏清理/错误谓词、健康协议、map key 自举根、真实
VM、全部。CLI 只使用固定 faults 和 solution，退出 0 不表示 starter 已完成。

关键观察：

| 边界 | Heap 实际可访问 | map entries | File owner |
| --- | --- | --- | --- |
| 初始 | T T T T | T T T T | T F T F |
| Sweep 完成、retain 之前 | T T F F | T T T T | T F T F |
| 按 liveness retain 之后 | T T F F | T T F F | T F F F |
| 撤掉合法 roots 后再完成一轮并 retain | F F F F | F F F F | F F F F |

模型使用真实 Heap、File 和 HashMap，不是布尔模拟。每项同时核对 `is_live` 和
`userdata_is_finalized` 的实际访问结果，保留在 Rust 中的四个 handle 一直没有删除。
`false` 可访问性与 `is_live == Ok(false)` 联合证明对象已不可访问，并不是只读 object ID。

## 一道 TODO

输入 `&mut FileModel` 是宿主资源的独占借用，`&GcStep` 是已经取得的本次步进结果。
只有 `step.completed.is_some()` 时，按 `heap.is_live(*userdata)` 恰为 `Ok(true)`
保留 map 项。把 heap 共享借用和 files 可变借用拆到不同字段，短借用只覆盖 retain。
关闭状态是 Option，收集状态由 Heap 决定；不能用 `file.is_some()` 代替 liveness。

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_prune_exercises completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones -- --ignored --exact
```

初始应恰好运行 1 项并失败。完成标准包括：不完整周期不删 map、完整周期删除两个
不可达项、保留可达 `None`、撤根后全回收。Windows 上还要让 exclusive reopen 真正
从失败变成功。只比较 map 长度会漏掉“删了活 tombstone、却保留死 open”的错误。

固定错误 `faults::forget_prune` 使 userdata 已 stale 但 OS File 仍占用；
`faults::prune_closed_instead_of_dead` 同时误删活 tombstone、漏掉死 open。
`FileModel::finish_cycle(true)` 把所有 map key 注入 roots，即使合法 roots 已清空，
全部 userdata 仍存活；此时正确 retain 也无法释放它们。这是 GC roots 输入错误。

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_prune_reference completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_prune_reference real_vm_host_gc_clears_weak_slots_and_releases_os_handles -- --exact
```

每条应恰好运行 1 项并通过。默认参考共 6 项，覆盖合同、三个固定错误、不完整周期和
真实 VM；默认 learner 共 1 项 ignored。参考通过和 ignored 不算 learner 完成。

## 步进、回调和证据边界

模型预算为 1，真实经过一个 `completed=None` 的 Sweep 步骤：第三个 userdata 已经
移除，第四个还在，map 此时仍保留四项。合同要求等完成周期才 prune。这不表示每次
提前 retain 都立即破坏活对象：`is_live` 检查 Heap 中对象是否存在，不是标记位；
未被 Sweep 删除的对象仍返回 true。

生产 `Vm::gc_step_internal` 先取得 Heap step、接收并执行 finalizer batch，然后才
根据**之前捕获的** `step.completed` prune，最后恢复临时 roots 并返回首个回调错误。
因此不要改成根据回调之后的瞬时 phase 判断。此局部模型没有 finalizer 或回调重入；
若 userdata 有 finalizer，pending 及 resurrection 可能使它多活一轮，不能从这里
推出所有 finalizable 文件第一轮就释放。当前实际文件 fixture 没有注册 finalizer。

`real.rs` 连续使用同一 VM，多次 execute 都只向宿主返回 bool。创建帧结束后才调用
host `collect_garbage()`，避免用尚在寄存器中的局部 handle 偶然保活。根保留的 open
文件实做 write/flush；closed tombstone 实做方法查找并验证 closed-file 错误。弱表中
不可达项精确为 nil；清掉全局 owner、再次结束 execute 并 host GC 后，四槽均为 nil。

Windows 探针用 `OpenOptionsExt::share_mode(0)` exclusive reopen：持有 File 时必须
得到 sharing violation 32，prune 后必须实际打开成功；其他错误会让验收失败。
删除文件成功不等于句柄关闭，因此没有用 remove 成功作释放证据。非 Windows 输出
`Unsupported`，只运行 Heap/VM 合同，不宣称普通 reopen 具有等价 OS 释放证明。
临时目录使用每次唯一名称，显式清理会合并原始错误与清理错误；Drop 仅提供不 panic
的后备清理。

理解题：为什么活的 `None` 仍要留在 map？调试题：若把 keys 全放 roots，为什么
retain 看起来“工作正常”却永远删不到项？完成后应能沿 Heap Sweep → 宿主 retain →
File Drop 三个边界逐项说明状态；本题不证明所有 GC 路径、panic、I/O 错误或整个 VM
资源生命周期已经完备。
