# v0.16：受保护调用返回的是一个完整结果协议

先预测两件事：

1. body 返回 `7,nil,9,nil` 或一个值都不返回时，pcall 的结果分别有几个？
2. xpcall 的 body 成功时，要不要运行 handler？失败后 handler 返回 `42,99`、无结果、
   `nil,99`，或 handler 自己报错，又分别应该交回什么？

本包只修改 [starter.rs](starter.rs) 两个独立 TODO。模型拿到**已经执行完的 body
Result**，只研究结果打包和 handler 消费协议；它不执行 Lua、管理 GC roots、保存
continuation 或修改 yield depth。另用 [real.rs](real.rs) 执行未修改的真实 VM 对照。

## 先重放固定错误与参考设计

从仓库根目录执行：

```powershell
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example protected_problem_chain -- pcall
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example protected_problem_chain -- xpcall
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example protected_problem_chain -- real
```

无参数或 `all` 会重放全部；错误参数退出 2。CLI 固定调用 faults/solution，不读取
starter。即使 TODO 修好，固定 before 仍显示错误；退出 0 不能说明学习者已经完成。

```text
before pcall first-only=[Boolean(true), Number(7.0)]
after pcall complete=[Boolean(true), Number(7.0), Nil, Number(9.0), Nil]
after pcall empty=[Boolean(true)]
before always-handler: calls=1 values=[Boolean(false), Number(42.0), Number(99.0)]
after conditional: calls=0 values=[Boolean(true), Number(7.0), Nil, Number(9.0)]
after conditional body-error=[Boolean(false), Number(42.0)]
after handler-empty=[Boolean(false), Nil]
```

固定错误版是独立教学假设，不代表真实 VM 当前正丢弃 pcall 多值或在 body 成功时
调用 handler。真实实现见 `crates/lua_vm/src/lib.rs` 的 `BaseFunction::PCall/XPCall`。

## TODO 1：结果 Vec 不能压成一个 Option

`pack_pcall(Result<Vec<Value>, VmError>) -> Vec<Value>` 消费 Result：

| body outcome | 必须返回 |
| --- | --- |
| `Ok([])` | `[true]`，只有一个结果 |
| `Ok([nil])` | `[true,nil]`，两个结果 |
| `Ok([false])` | `[true,false]`，false 是普通成功值 |
| `Ok([7,nil,9,nil])` | `[true,7,nil,9,nil]`，五个结果 |
| `Err(error)` | `[false, formatted_error_string]` |

`first_result_only` 只取 `values.first()`，既丢掉中间/尾部结果，又把“无结果”错变成
“一个 nil”。只看固定数量的 Lua 局部变量可能看不出后一种差别，因为赋值本身会补 nil。
因此真实探针用 `select('#',...)` 和开放结果一起检查**数目与内容**。

```diff
- vec![Value::Boolean(true), values.into_iter().next().unwrap_or(Value::Nil)]
+ let mut results = vec![Value::Boolean(true)];
+ results.extend(values);
+ results
```

不筛掉 nil，也不把 `Ok([false])` 当成失败。`Vec::extend` 移入全部 Value，保持顺序；
错误分支则把 VmError 的格式化文本复制成拥有型 ByteString。

```powershell
# 初始执行 1 项失败；只修 pack_pcall 后同一项通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_exercises pcall_keeps_the_complete_result_window -- --ignored --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_reference pcall_keeps_the_complete_result_window -- --exact
```

## TODO 2：handler 是失败分支，不是成功结果的后处理

`pack_xpcall(outcome, &mut handler)` 不依赖你在 TODO 1 写的函数，可以独立修复。
handler 是暂借的 Rust `FnMut(Value) -> Result<Vec<Value>, VmError>` 协议探针，允许合同
记录调用次数与收到的消息。它不冒充真实 Lua closure 的 owner 或 GC root。

处理规则如下：

- body 成功：前缀 true 后保留全部结果，handler 调用 **0 次**。
- body 失败：把错误格式化为一个字符串，调用 handler **恰好 1 次**。
- handler 成功：只取第一个结果，类型可为 number、bool、table 或 nil；无结果补 nil。
- handler 失败：选择 handler 自己的 VmError 文本。
- body 已失败，所以最终前缀始终为 false，不能因为 handler 成功而改成 true。

```diff
- // 不论 body 成败都调用 handler，再展开 handler 全部结果。
+ match outcome {
+     Ok(values) => { /* true + 原来全部 values；不调用 handler */ }
+     Err(error) => {
+         let handled = handler(error_text(error))
+             .unwrap_or_else(|e| vec![error_text(e)]);
+         vec![Value::Boolean(false), handled.into_iter().next().unwrap_or(Value::Nil)]
+     }
+ }
```

`nil,99` 的首结果仍然是 nil，不应继续寻找第一个非 nil；`42,99` 的 99 也不应泄漏
到 xpcall 的第三个返回槽。共享合同同时检查这些值、精确结果长度、handler 次数及
它收到的单个格式化字符串。需要答案时再看 [solution.rs](solution.rs)。

```powershell
# 初始执行 1 项失败；只修 pack_xpcall 后同一项通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_exercises xpcall_only_handles_failures_and_selects_one_result -- --ignored --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_reference xpcall_only_handles_failures_and_selects_one_result -- --exact
```

## 真实 VM 对照：在同一次 execute 中继续运行

默认参考有 9 项：两个共享合同、两个固定错误有效性、五项真实 VM 对照。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_exercises -- --ignored
```

第二条初始是 2 项失败；两题完成后必须 2 项通过。默认 workspace 的 `2 ignored`
不表示完成。不要改合同、固定故障、真实 VM、默认 ignored 或预期结果。

真实 VM 分别验证：

1. pcall 多结果含尾部 nil，开放 relay 得到 `5,true,7,nil,9,nil`。
2. body 接收 table 参数、GC 后报错；同一 execute 捕获消息，离开观察块后再 GC，
   `weak.argument` 精确为 nil；随后另一个 pcall 仍能读取 prefix 算出 42。
3. xpcall 成功保留全部值，handler 次数为 0。
4. body 失败时 handler 仅运行一次，首结果/空结果/首 nil/handler 错误均符合规则；
   handler 返回 table 后，写回调用者再 GC，读取 `object.answer` 仍为 42。
5. `error(table)` 的消息和传给 handler 的值都是 string；不可调用的 body 也可被 pcall
   捕获。原 payload 表仍可访问，但不是直接作为错误返回的 Value。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_reference real_vm_continues_after_error_in_the_same_execute_and_releases_argument -- --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_reference real_vm_xpcall_handles_first_empty_nil_and_handler_errors -- --exact
```

这些正向探针都在**各自同一次 execute** 内消费错误并继续，没有借用下一次 execute
重置 VM 状态来证明清理。weak 参数观察也只证明此参数最终未被保留，不能据此声称
所有私有 temporary roots 都逐项准确恢复；精确 `root_base/truncate` 仍需读真实源码。

## 必须校准的所有权与时点

真实 `invoke` 在调用前记录 `temporary_roots.len()`，把 callable/arguments 作为
临时 roots，通常 Result 路径在调用返回后截回前缀；body 的活动 slots 和调用者的正常
roots 另有来源。`CoroutineWrapper` 分支中的 `resume_coroutine(...)?` 仍可能提前
离开而跳过尾部 truncate。本结果模型没有复制这套机制，也没有证明全部回调或错误
路径都能精确恢复 roots；该边界不能被此处九项默认通过覆盖。

handler 返回值到 XPCall 选择首结果、再到外层 CALL 写回寄存器之间，没有额外 GC
checkpoint。返回的 `Vec<Value>` 本身不是自动注册的根；不能在该间隙擅自插入 GC，
再把本例当作已有保活证明。真实 table 结果测试检查的是**写回后**再 GC 的存活。
禁止 yield 的深度恢复由另一个实验包处理，此处不重复模拟；也不外推 panic/abort 清理。

这份快照把 `error(...)` 转成 VmError 的字符串消息，pcall/xpcall 再使用其格式化文本。
不要把这里的协议说成保存了任意 Lua 原始错误对象，或宣称完整官方错误对象兼容。
xpcall handler 成功时则可以返回非字符串首结果，这与 body 错误已字符串化是两回事。

理解题：`[]`、`[nil]` 与 `[false]` 为什么必须分开？
实现题：独立完成两个 TODO，跑两条显式 learner 命令。
调试题：handler 返回 `nil,99` 时，为什么不能过滤 nil 或把 99 展开出去？

本机局部证据保存在 `target/teaching-v16-callbacks-2026-10-15/protected/`，为忽略
产物，不随 clone 交付。永久入口是本 README、contracts、faults、solution 与真实探针。
