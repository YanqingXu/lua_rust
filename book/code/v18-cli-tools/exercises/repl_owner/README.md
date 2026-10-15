# 为什么下一行 REPL 还看得到上一行创建的对象？

先预测：把每一行都交给 `lua_app::execute_multi(source)`，是否等同于交给同一个
`Session::execute_multi(source)`？本包只有一道 TODO，修改 `starter.rs::submit`，
学习让输入短借用同一个长期 owner。使用真实 Session/VM，没有另造解释器模型。

从仓库根目录运行固定回放：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --bins
cargo run --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_owner_problem_chain -- all
```

CLI 接受 `owner|roots|cli|all`，缺省 all。owner 对比 fresh/shared，roots 观察宿主
句柄与 GC roots，cli 启动真实 lua_app 检查多输入。它不读取 starter；固定回放退出
0 和默认 learner ignored 都不代表 TODO 已完成。

## 不是“再次编译”，而是谁还拥有 VM

`lua_app::execute_multi` 内部每次创建 `Session::new()`，执行结束后这个 Session
随调用返回而销毁。每个输入单独编译仍然可以正确；上一输入的全局状态却属于另一个
已经结束的 owner。参考方案由调用者创建 Session，逐次用 `&mut Session` 短借用。

```diff
-pub fn submit(_session: &mut Session, source: &[u8]) -> Result<Vec<Value>, AppError> {
-    lua_app::execute_multi(source)
+pub fn submit(session: &mut Session, source: &[u8]) -> Result<Vec<Value>, AppError> {
+    session.execute_multi(source)
 }
```

输入是借用的字节串，输出仍为多返回值 `Vec<Value>` 或原来的 `AppError`，没有增加
错误包装。长期持有者是 Session 的私有 Vm；一次调用结束只结束这次可变借用，不会
销毁 Session。`Vm::execute_multi` 每次重置活动帧、活动函数和临时 roots，并新建
chunk function，保留同一个 global environment、package 缓存和 registry owner。

## 共享合同：实际读取什么

`chunks.rs` 的 setup 返回 `[10,40,3,7,1,9,123]`，全为标量。下一输入先 `gc_collect()`，
再观察七个位置：

| 观察位置 | fresh 每次新建 | 同一个 Session |
| --- | --- | --- |
| global `repl_number` | nil | 10 |
| global table 的 `answer` | nil | 40 |
| global closure 捕获的 local table 中 `n`，执行加一 | nil | 4 |
| `package.loaded.repl_owner_mod.answer` | nil | 7 |
| loader 执行次数 | nil | 1 |
| registry 独有 table 的 `answer` | nil | 9 |
| 上一 chunk 的 `local chunk_only` | nil | nil |

第三输入修改同一图，再 GC 并读取 `[11,42,5,true,8,1,10,nil]`；其中 true 验证
`require` 返回的缓存表身份，随后实际访问 answer=8，loader 仍只执行一次。闭包持有
的 local cell 通过 global closure → upvalue → table 保活；普通 chunk local 不会
自动变成下一输入的词法绑定。只把全局变量保留、但重置缓存或 registry 的实现也不能
满足这个合同。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_owner_exercises submissions_preserve_globals_tables_closures_and_owner_edges -- --ignored --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_owner_reference submissions_preserve_globals_tables_closures_and_owner_edges -- --exact
```

每条必须恰好运行一项。初始 learner 应失败；完成 TODO 后，同一共享合同通过。
`faults::fresh_session` 是固定故障，始终使用 fresh helper，避免 learner 的修改把
反例一并“修好”。所有 Session 合同只向宿主返回数字、布尔和 nil，跨 GC 的对象访问
来自实际 global/package/registry edges，不依赖打印 handle 或宿主保留上一次 frame。

## Rust 拿到 Value，不等于 GC 获得 root

Session 的 owner 保留 VM，并不意味着任意 Rust `Vec<Value>` 都自动注册到 GC。
`real::copied_host_handle_is_not_a_root` 单独使用公开 Vm API：返回 table 的 Rust
Value 仍留在宿主，先执行另一个只返回 true 的 chunk，清掉上次执行的临时结果根，
再 host GC。尝试把旧 Value 写入 registry 必须精确得到 `HeapError::StaleHandle`。

第二个 table 在 GC 前显式写入 registry，GC 后验证同一 handle，并通过 Lua 真正
读取 `debug.getregistry().repl_owner_host.answer == 42`。删除 registry 项、结束另一次
只返回 true 的输入，再 host GC，registry slot 精确为 nil；宿主仍保存的旧 Value
再次写入 global 时得到 StaleHandle。两次拒绝是带 Heap 验证的真实访问，不能仅用
“Rust 中 handle 还存在”或 object ID 文本判断对象存活。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_owner_reference copied_host_values_need_vm_roots_and_are_stale_after_release -- --exact
```

这个 probe 与 Session 合同分开：Session 不公开私有 Vm，不能假装通过 Session
直接调用了 registry_set。它只说明这里测试的简单 table 图和同步边界，不证明任意
Value 逃逸、活动 varargs、sort snapshot、重入或所有 GC 路径已经安全。

## 真正的命令行也必须共享 owner

两项集成测试使用 `env!("CARGO_BIN_EXE_lua_app")` 的本次构建 binary，不从 PATH
选择另一程序。示例用 current_exe 定位同 profile 的 sibling binary，所以运行示例
前先 `cargo build --bins`；使用自定义 target-dir/profile 时，两条命令保持一致。

第一项用两个 `-e` 执行 setup 和 mutation，再运行临时脚本。脚本再次 GC、检查
chunk local 为 nil、模块身份相同，实际打印字段：

```text
script    11    42    5    8    1    10
```

列之间是 tab，closure 的 5 来自 setup/第二个 `-e`/script 共调用三次；此路径没有
运行合同的 observe 输入。stdout 精确匹配，stderr 必须为空，退出码必须为 0。

第二项先 `-e` setup，再 `-i` 接收三行：读取 table/closure/module/registry，修改
number 和 table，然后 GC 并重新读取。带真实 REPL prompts 的结果依次包含
`10\t40\t4\t7\t9\tnil`、`true`、`11\t42\t5\ttrue\t1`。这些字段来自不同输入
的实际执行，没有把 table/function 的展示字符串当成存活或身份依据。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_owner_reference real_cli_multiple_e_actions_then_script_share_session -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_owner_reference real_cli_e_action_then_interactive_inputs_share_session -- --exact
```

共享 support/process helper 并发读取 stdout/stderr，写完输入即关闭 stdin，并以五秒
deadline 等待此 child；失败会 kill/wait 同一个 child。此组 fixture 不启动后代进程。
临时脚本目录每次唯一，显式 finish 合并原始错误和清理错误，清理前校验 canonical
范围；Drop fallback 不 panic。测试不修改进程 cwd 或全局环境。

默认参考共 **5 项**（共享合同、fresh 固定反例、宿主 roots、两条真实 CLI），learner
共 **1 项 ignored**。理解题：为什么 chunk local 不跨输入，捕获它的 closure 却能
跨输入？调试题：如果全局表能读到，而 require 重跑或 registry 字段丢失，应检查
owner 的哪一层？完成检查点是解释 Session → Vm → roots → 对象图，并让唯一 TODO
显式通过；REPL 语法续行和错误后的 buffer 恢复属于另一个练习包。
