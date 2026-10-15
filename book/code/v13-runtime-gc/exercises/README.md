# v0.13 GC 可执行练习

这组练习使用本目录的最终 v0.13 快照。从仓库根目录运行以下 PowerShell 命令。
先写下预测，再看输出；修改范围是 `cycle_starter.lua` 和 `starter.rs`，不用修改
生产 `crates/`。`contracts.rs` 是验收规格，`reference/` 是可选参考解。

```powershell
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_reference
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_exercises -- --ignored
```

第一条应有 6 项通过。第二条专门启用学习者的 TODO：初始有 4 项失败，完成练习后
应有 4 项通过。默认 `cargo test --workspace` 编译 starter，但跳过这 4 项；它们的
`ignored` 状态表示尚需读者显式验收，不能作为练习已完成的证据。

| 练习 | 只修改 | 验收名 | 初始观察 → 完成条件 |
| --- | --- | --- | --- |
| 理解：双表环 | `cycle_starter.lua` 的 TODO | `two_table_cycle_loses_its_last_global_root` | `false` → `true`，保留与移除 global 都符合预测 |
| 实现：宿主租约 | `starter.rs` 的 `pin` / `unpin` | `host_registry_lifetime` | registry 返回 `Nil` → pin 后可达、unpin 后旧 handle 被拒绝 |
| 调试：CLOSE 时机 | `starter.rs` 的 `restore_scope_exit` | `close_before_register_reuse` | 捕获值变成 `99` → 关闭后仍为 `10` |
| 调试：遗漏 primitive root | `starter.rs` 的 `primitive_roots` | `primitive_metatable_root_lifetime` | `StaleHandle` → 有 root 时 child 存活，删 root 后两者回收 |

单项验收把名称放在 `--` 前，把 `--ignored --exact` 放在后面，例如：

```powershell
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_exercises host_registry_lifetime -- --ignored --exact
```

## 理解题：环会不会让对象永远活着？

打开 [cycle_starter.lua](cycle_starter.lua)。画出两个时刻的图：只有 `left ↔ right`
时，以及 `global_root → left ↔ right` 时。预测第一次 `gc_collect()` 收回几个表；
global 保留时能否沿 `other.other` 回到自身；删除最后一条 root 边后能否回收整个环。

```powershell
cargo run --quiet --manifest-path book/code/v13-runtime-gc/Cargo.toml -- book/code/v13-runtime-gc/exercises/cycle_starter.lua
```

初始输出是 `type=boolean value=false`。只在 TODO 处结束 global 的保留，不删除环内
的两条边，输出应变为 `type=boolean value=true`。CLI 返回成功仅表示程序正常执行，
布尔值才是本练习的结果；对应 Rust 验收会把 `false` 当成失败。

`gc_live()` 数的是 arena 中的 table、closure、upvalue record，不是字节数；registry
和当前 chunk 的 closure 也在其中。因此脚本先回收，再记录相对基线。观察 global
身份的表达式放在 `do/end` 内：它产生的临时 table handle 也占寄存器，离开块时
`CLOSE` 清除这些槽，才能把下一次观察收敛到 global 是否仍存在。把这段 `do/end`
拿掉，可以进一步观察“Lua 局部名已经没用”为什么不等于“活动帧中已无 handle”。

完成后再与 [reference/cycle.lua](reference/cycle.lua) 对照，并解释为什么不需要
逐个断开 `left.other` 和 `right.other`。

## 实现题：让宿主 Value 拥有一个明确的保留期

`Value::clone()` 复制 table handle，不拥有 Heap record。[contracts.rs](contracts.rs)
使用 `compile` 和一个持续存在的 `Vm`，先证明宿主仍保留 alias 时，无 registry root
的返回表会被回收；尝试再次登记这个值会得到 `StaleHandle`。

接着实现 `pin(vm, key, value)` 和 `unpin(vm, key)`：

1. pin 必须发生在第一次 collection 之前，用既有 `registry_set` API 建立可达边，
   并把 API 的错误传给调用者。
2. unpin 结束同一 key 的保留期。最后一次 collection 后，宿主 alias 仍存在，表应
   不再存活；旧 handle 不能重新登记。
3. 失败的旧值登记不得破坏 registry。验收会继续写入数字并读回 `42`。

这里不能用 `lua_app::execute()` 做跨调用实验，因为它每次创建临时 VM，返回后
Heap 就随 VM 释放。参考解见 [reference/solution.rs](reference/solution.rs)，既有
VM 测试 `registry_keeps_host_values_alive_until_the_entry_is_removed` 则是较短的
registry 正例。

## 调试题：CLOSE 为什么必须在寄存器复用之前？

验收把编译后的 Proto 克隆成故障输入，将顶层的每个 `Close` 替换为 `Jump 0`。
这个跳转只走到下一条指令，保留 instruction index，不改变跳转目标。原程序返回
`10`，故障程序返回 `99`：捕获的 Open upvalue 仍读取被后一个 local 复用的栈槽。

在 `restore_scope_exit(original, faulty)` 中恢复作用域出口。不要改常量 `99`、
闭包结果或验收期望，也不要删除指令移动索引。验收先确认故障确实得到 `99`，
再要求修复输入恢复 `10`。本操作只修改练习中的 Proto 副本，重跑会重新生成故障。

还要解释 `CLOSE` 的两件事：先让 upvalue 保存退出时的值，再把退出作用域的槽
清成 `Nil`。后一项决定已失效的局部对象会不会继续被活动帧当作 root。已有回归可
用于分别阅读这两个条件：

```powershell
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test pipeline leaving_a_block_closes_before_register_reuse -- --exact
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test pipeline full_gc_reclaims_an_unreachable_self_referential_table -- --exact
```

## 调试题：primitive metatable 没有 table 对象帮它保留

`primitive_roots` 是 `Vm.type_metatables` 根收集的小模型，只接受 metatable handle。
`Vec::new()` 漏掉这些根，collection 会成功，却把之后还要使用的表误收，后续访问
返回 `StaleHandle`。实现根集合后，metatable 与其 child 应存活，不相关表应回收；
移除所有 type metatable 后，两张表都应回收。这个模型不代表 VM 完整 roots 清单。

完整 VM 的对应反例在 [primitive_metatable.lua](primitive_metatable.lua)：局部 `mt`
离开作用域后收集，再通过数字的 `__index` 读取 `bonus`。正确快照输出 `42`。

```powershell
cargo run --quiet --manifest-path book/code/v13-runtime-gc/Cargo.toml -- book/code/v13-runtime-gc/exercises/primitive_metatable.lua
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_reference primitive_metatable_remains_usable_after_scope_and_collection -- --exact
```

如需验证真实遗漏，只在教学快照的
[`crates/lua_vm/src/lib.rs`](../crates/lua_vm/src/lib.rs) 中找到 `collect_garbage()`，
临时注释这一行，然后运行上面的定向测试：

```rust
roots.extend(self.type_metatables.values().copied().map(Value::Table));
```

应从 `42` 变为带 `stale GC handle` 的 `VmErrorKind::Metamethod` 错误。保存修改前的
文件或用编辑器撤销，再恢复这一行并重跑，同一项应通过。不要用 `git checkout`
覆盖文件里的其他修改。CLOSE 的默认调试练习不需要这样修改 VM。

## 失败恢复验收：mark 出错以后能否重试？

新增参考验收 [reference.rs](reference.rs) 的
`mark_failure_preserves_heap_and_clears_partial_marks` 不需要填 TODO，可逐项阅读：

```powershell
cargo test --manifest-path book/code/v13-runtime-gc/Cargo.toml --test gc_reference mark_failure_preserves_heap_and_clears_partial_marks -- --exact
```

持有 Open upvalue 栈槽的 `RefMut`，让 marker 在已经标记了 closure、Proto 常量图
等对象后遭遇 `StackBorrowConflict`。本轮必须返回错误且不执行 sweep，全部 6 个
record 仍在；随后解除借用，为 parent 加一个新的 child，并换成只有 parent 的根集合。

重试应收回 4 个旧 record，保留 parent 和两个 child。若错误路径没清除部分 mark，
已失去 root 的 abandoned 会假存活；parent 也会跳过新边，误收新 child。最后空根
collection 把剩余 3 个 record 收回。这验证的是可恢复失败边界，不能外推为自动 GC、
OOM、weak table 或 finalizer 已实现。
