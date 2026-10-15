# close 后 File 该消失，Lua userdata 为什么仍然存在？

先预测：`alias=file; io.close(file)` 之后，`alias` 还是同一个 userdata 吗？
`io.type(alias)` 应该返回 nil，还是 `"closed file"`？再次 close 与传入非文件值的
错误类型是否相同？本包只设 **一个 close TODO**，登记时已经用 move 建立唯一 owner。

从仓库根目录运行：

```powershell
$moduleManifest = 'book/code/v17-host-stdlib-modules/Cargo.toml'
cargo run --quiet --manifest-path $moduleManifest --example file_close_problem_chain
cargo test --manifest-path $moduleManifest --test file_close_reference
cargo test --release --manifest-path $moduleManifest --test file_close_reference
```

CLI 缺省为 `all`，也接受 `close`、`identity`、`real`。它只使用固定 fault 与 solution，
不读取 starter。默认有 8 项参考测试；唯一 learner 默认 ignored，初始显式执行应失败。
退出 0 只说明固定对照重放成功，不能代替 learner 验收。

## 一个身份，两种所有权

真实快照将 `File` 放在 `HashMap<UserdataRef, Option<File>>`；本模型使用相同 key/
Option 结构，外包一层 `OwnedFile` 记录 Drop 次数，内部仍是真正的 `std::fs::File`。
登记函数消费 File，没有 `try_clone`，复制 Lua `Value::Userdata` 只复制 typed handle。

| map 项 | io.type | 宿主 File owner | Lua userdata / metatable |
| --- | --- | --- | --- |
| 没有该 key | nil | 没有 | 可能存在，但不被此 map 识别为文件 |
| Some(OwnedFile) | `"file"` | map 唯一拥有 | 由 Lua roots / Heap edges 决定可达性 |
| None tombstone | `"closed file"` | 已经释放 | 仍可存活，并保有 metatable edge |

close 只把第二行变成第三行。它没有删除 Lua userdata、清空 metatable 或回收所有别名。
模型在 close 后用同一个 userdata 作为唯一 Heap root，完成 GC 后实际沿 metatable
读到 `metadata.answer=42`；这个读取与宿主 File 是否关闭是两个独立观察。

## 固定反例：flag 和 remove 分别漏掉什么

在 Windows 的 `close` 输出中：

```text
before flag-only: ... io.type=Some("file"), drops=0, claimed=true, ... exclusive=Some(false)->Some(false)
before remove-entry: ... io.type=None, drops=1, claimed=false, repeat=Err(NotFile), ...
after take-owner: ... io.type=Some("closed file"), drops=1, ... repeat=Err(AlreadyClosed), ...
```

flag-only 故障引入一份额外的“声称关闭”记录，但仍把 File 留在 Some，OS handle
继续被拥有，exclusive probe 继续失败。健康实现不需要第二份 bool 状态。
remove 故障确实释放了 File，却让这个仍存活的 userdata 失去 closed-file 分类，
再次操作会被误报为 NotFile。它不是释放资源失败，而是丢了观察身份。

## 唯一 TODO：取走 Some，留下 None

只修改 `starter.rs::close(&mut FileVm, UserdataRef) -> Result<(), FileError>`。
`get_mut` 暂借 map 项，`Option::take` 把拥有的 File 移出并原地写 None；局部 owner
随后 drop。借用不跨 loader/callback，本题没有 callback 或异步关闭。

```diff
-let owned = vm.files.remove(&handle).ok_or(FileError::NotFile)?;
-drop(owned.ok_or(FileError::AlreadyClosed)?);
+let entry = vm.files.get_mut(&handle).ok_or(FileError::NotFile)?;
+let owned = entry.take().ok_or(FileError::AlreadyClosed)?;
+drop(owned);
```

真实 VM 用 `file.take().is_none()` 完成同一所有权动作；Some 临时值在这条语句结束时
drop。模型显式命名 owner，便于观察何时释放。Rust File Drop 不报告 close 系统调用
的错误，本包不外推“数据已持久化”或可恢复的 flush/close 事务。

```powershell
cargo test --manifest-path $moduleManifest --test file_close_exercises close_releases_owner_but_keeps_closed_identity -- --ignored --exact
cargo test --manifest-path $moduleManifest --test file_close_reference close_releases_owner_but_keeps_closed_identity -- --exact
```

共享合同在关闭前真实写入/flush/seek/读取 `A\0\xff`，关闭后检查 Drop 恰好一次、
None tombstone、别名身份、metadata 的实际 GC 后访问，以及全部错误分支：

- 重复 close：AlreadyClosed / Runtime；read/write/seek/flush：UseClosed / Runtime。
- 同一真实 Heap 中分配、却从未登记到文件 map 的 userdata：NotFile / Type，
  `io_type` 为 None。closed-file 与“不是文件”不能合并。
- Windows 独占 reopen 从失败变成功；非 Windows 不执行这项 OS 证明。

## 为什么“删除成功”不是关闭证据

删除只是清理临时路径。Unix 可以 unlink 仍打开的文件；Windows 的 Rust 默认打开
也可以允许 delete sharing，所以删除成功不能推出旧 File 已释放。

`fixture::exclusive_available` 仅在 `cfg(windows)` 使用
`OpenOptionsExt::share_mode(0)` 以 read+write 独占重新打开同一路径。先在文件确实打开
时观察 sharing/lock violation，再在 close 后成功重新打开并立即 drop 探针 File。
这比删除严格，也避免只看 bool 或 Drop 计数就声称实测了 OS sharing。

非 Windows 返回 None，CLI 明示 unsupported；portable 的真实 File IO、Drop 计数、
tombstone、GC 与 VM 错误合同照常执行。它们不能代替 Windows 独占重开证据。

## 真实 VM：只通过公开行为验证

`real.rs` 不读取 VM 私有 files map。四项真实参考分别检查：

- 同一文件的两个 Lua alias：`io.close(first)` 后 alias 是 closed file，再 `alias:close`
  报错；另一文件反过来用 `f:close()` 后 `io.close(alias)` 报错。
- 同一 VM、同一个已关闭文件：io.close/file:close/read/write/seek/flush 共 6 项是
  VmErrorKind::Runtime，消息分别是 close/use a closed file；把 table 传给 io.close
  或保存的 5 个 native methods，另外 6 项是 Type，消息含 `expected userdata`。
  模型另有“真正 userdata 不在 map”的 NotFile 探针，真实 Lua 的这组错误输入是 table，
  不能把两者声称为同一条消息路径。
- 为文件 userdata 设置独有 metatable，`__index` 指向真实 file methods，metadata 只由
  该 userdata 的 metatable edge 持有。关闭并结束 setup frame，host GC，再次 execute
  实际读取 `metadata.answer=42`，而 `io.type` 仍为 closed file。
- Windows 在真实 VM 打开文件后独占重开失败；通过 alias 调 io.close 后，VM 和 Lua
  userdata 仍活着时独占重开成功。没有用 Vm Drop 掩盖显式 close 的释放时点。

```powershell
cargo test --manifest-path $moduleManifest --test file_close_reference real_vm_io_close_and_file_close_share_alias_state -- --exact
cargo test --manifest-path $moduleManifest --test file_close_reference real_vm_closed_operations_are_runtime_errors_and_wrong_types_are_type_errors -- --exact
cargo test --manifest-path $moduleManifest --test file_close_reference real_vm_closed_userdata_retains_its_metatable_edge_across_execute_and_gc -- --exact
cargo test --manifest-path $moduleManifest --test file_close_reference real_vm_windows_exclusive_reopen_observes_file_release -- --exact
```

临时目录名包含进程号、时间与原子序号，create_dir 拒绝接管已有目录；清理前
canonicalize 并核对唯一子目录名、父目录与前缀。正常路径显式 finish 检查清理，
提前 unwind 的 Drop fallback 只报告失败，不再 panic。probe 不改 cwd 或全局环境。

本模型不是历史 VM，也不覆盖 stale 文件条目的 GC prune、VM shutdown 顺序、IO
平台错误恢复或任意重入。模型 map key 本身不自动注册 Heap root；保活合同显式传入
仍活 alias。下一包再讨论 userdata 不可达时宿主 map 为什么还拥有 File。

理解题：为什么 File 已 drop，userdata/metatable 仍能通过 GC？
实现题：只完成 close hook，显式 learner 运行恰好 1 项并通过。
调试题：对照 flag-only 与 remove，解释 drops、io.type 和 exclusive probe 为什么
给出不同证据；再解释为什么删掉测试文件不能证明任意平台的 OS handle 都已关闭。
