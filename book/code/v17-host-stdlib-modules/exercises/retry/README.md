# v0.17：第一次 require 报错，为什么第二次却“成功”了？

先预测：require 在调用 loader 前写入 `loaded[name]=true`，loader 用 `?` 返回错误后，
这条 true 还在吗？修好 loader 再调用 require，它会重新执行，还是直接命中旧 true？

本包只修改 [starter.rs](starter.rs) 的一个 `attempt` TODO。模型只覆盖**loader 已经
选好、loading=true 已写入之后**的普通 Result 协议；文件搜索、编译、yield 深度与
Heap roots 由真实 VM 探针另行核对。它不是历史 VM 中间快照，也不修改生产加载器。

## 固定反例：`?` 先于失败清理返回

从仓库根目录执行：

```powershell
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_retry_problem_chain -- retry
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_retry_problem_chain -- dependency
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_retry_problem_chain -- real
```

无参数或 `all` 重放全部；错误参数退出 2。CLI 固定调用 faults/solution，不读取 starter。
退出 0 表示固定对照执行完成，不表示学习者完成；修复 TODO 后 before 也仍展示错误。

```text
before question-mark: first_failed=true cached_after_failure=true
before question-mark: retry=true runs=1 effects=1
after current-key: first_failed=true cached_after_failure=nil
after current-key: retry=42 runs=2 effects=2
before clear-all: parent=nil dependency=nil outer=nil
before clear-all: retry=42 dependency_runs=2
after current-key: parent=nil dependency=41 outer=900
after current-key: retry=42 dependency_runs=1
```

错误版第一次确实报错，第二次却根本没运行修复 loader，所以 runs 仍为 1，结果是旧
loading 标记 true。不能只看第二次没有 Err；必须同时观察返回值与真正执行次数。

## TODO：先取得 Result，清理当前 key，再交付错误

接口是 `attempt(vm, name, loader) -> Result<Value, VmError>`。driver 已经完成 truthy
缓存检查并写入 true；hook 才开始执行 loader。普通成功继续走已给出的 `finish`，
只有 Err 需要先删除本次 name，再把**原 VmError**交回调用者：

```diff
- let results = loader(vm, name)?;
- Ok(vm.finish(name, results))
+ let result = loader(vm, name);
+ match result {
+     Ok(results) => Ok(vm.finish(name, results)),
+     Err(error) => {
+         vm.loaded.remove(name);
+         Err(error)
+     }
+ }
```

`loader` 暂借整个 `&mut RetryVm`，可递归 require 依赖、修改 cache 与其他数据；不能
持有某个 map entry 的可变借用后再调用它。取得拥有型 Result 以后才重新修改 loaded，
避免把清理写在 `?` 已经离开的路径之后。

```powershell
# 初始执行 1 项失败；完成唯一 TODO 后同一项通过。
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_exercises failure_removes_only_the_current_loading_entry -- --ignored --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference failure_removes_only_the_current_loading_entry -- --exact
```

`finish` 保持快照的现有成功协议：只选首返回值，非 nil 写入 cache；nil/空结果则重新
读取 loader 可能修改过的 loaded。这题不用重新实现成功缓存，也不改变循环加载策略。
需要参考时再看 [solution.rs](solution.rs)。

## 为什么不能恢复旧值，或清空所有模块？

共享合同分别从“原来没有 entry”和“原值 false”开始，并让 loader 选择直接报错或
先发布 77 再报错。四种情况下失败都必须让当前 key 读到 nil：

- 失败发生在 loading 标记以后；原来的 false 不恢复。
- loader 在当前 key 发布过值也不能把失败留成成功缓存。
- `calls/side_effects` 不回滚：清理的是本次缓存身份，不是事务回滚。
- 其他 key 的成功缓存继续保留；修好的 loader 真正重跑一次，随后才命中成功 cache。

另一个固定错误版用 `loaded.clear()`。A 加载 B 成功后 A 报错，它既清了 A，也删掉
成功依赖 B 和早已存在的 outer=900；重试时 B 被执行第二次。健康实现只删 A，B 的
模块值 41 与一次执行记录保留。这里的 outer 是已有 cache entry，不是 GC root 模型。

合同还精确比较返回的 VmError，包含 kind、pc、line、message、traceback，防止把原
loader 错误换成“重试失败”等新消息。模型的 BTreeMap 删除不会失败；真实 Heap 清理
API 自身若失败，源码会返回 cleanup 错误，本模型不宣称覆盖该额外错误路径。

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference faulty_question_mark_turns_retry_into_a_cached_true -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference faulty_clear_all_discards_successful_dependency_and_outer_cache -- --exact
```

## 真实 VM：失败、修复和重试必须共享上下文

[real.rs](real.rs) 的三组 preload 探针均在**同一次 execute**中继续：

1. preload 首次先修改 `package.loaded[name]` 再 error；观察失败后精确 nil、其他副作用
   保留；替换 preload，require 得到 answer=42，再 require 命中同一表，总执行两次。
2. A 依赖 B，B 成功返回表，A 随后发布 partial 并报错；只清 A，修好 A 后 B 不重跑，
   仍能实际访问 B.answer=41，已有 outer=900 也保留。
3. 同一 coroutine 中 require 的 loader 尝试 yield(99) 被拒绝，消息含 yield、当前
   loaded entry 被清；替换 loader 后 require 成功，再进行合法 yield(true)，下一次
   resume 输入 42 后返回 43。不能把先前被拒绝的 99 泄漏为外层 yield 值。

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference real_vm_preload_failure_published_value_is_removed_before_same_execute_retry -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference real_vm_parent_failure_keeps_a_successful_dependency_cached -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference real_vm_require_rejects_yield_then_restores_a_later_legal_yield -- --exact
```

`invoke_yield_forbidden` 先配对恢复 depth，再把 loader Result 交给 require 的清缓存分支。
这验证 require 新路径之后的可见状态恢复，不把上一章 pcall 的通过直接套作 require
证据；也不据此宣称所有私有 temporary roots 都精确恢复或 panic/abort 安全。

## 文件错误有两个不同的缓存时点

三个文件探针各自创建私有临时目录，用绝对 `package.path`，不改变进程 cwd。每个探针
始终使用**同一个 Vm**，失败后修改磁盘文件，再通过下一次 execute 重试；不重新建 VM
或重置 package tables 来制造“能重试”。清理前 canonicalize 目标，核对父目录、唯一
目录名与前缀；正常返回显式 finish 检查清理结果，提前 panic 的 Drop 只作不 panic 的
fallback，避免清理错误覆盖原故障。不会接管或删除已有同名目录。

| 场景 | loader 执行前的状态 | 失败后的 loaded 原 false | 修复后 |
| --- | --- | --- | --- |
| 文件不存在 | 尚未选出 loader，没写 loading 标记 | 保留 false | 创建文件，同 VM 加载成功 |
| 首个可读文件编译失败 | compile_source 的 `?` 返回，没写 loading 标记 | 保留 false | 修好该文件，同 VM 加载成功 |
| 文件编译成功，执行时报错 | 已写 true，开始执行 loader | 清成 nil | 改写文件，同 VM 重跑并缓存 |

**读到文件后，编译失败会直接退出 require，不继续尝试后面的 path template。**
编译探针特意把坏文件放在第一个目录、有效 fallback 放在第二目录；失败后 fallback
执行计数仍为 nil，修好首文件后也只执行首文件。源码的 `compile_source(...)?` 解释
为何搜索立即结束；探针没有把“后面的文件没执行”当成完整文件系统访问跟踪。

runtime 探针还检查副作用计数从 1 到 2、返回 table 的 answer=42、GC 后继续访问以及
第二次 require 命中相同模块值。读文件/编译失败不能与 loader 已运行后的失败混为
同一个 rollback 时点；旧 false 的两个不同结果正是可观察区别。

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference real_vm_missing_file_can_be_created_and_retried_without_replacing_vm -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference real_vm_compile_error_stops_before_later_template_then_retries -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference real_vm_runtime_failure_clears_marker_then_rewritten_file_retries -- --exact
```

本快照要求文件分支的模块名和 path 可解码为 UTF-8；这里没有扩大为任意平台路径
编码、C module loader 或 package.loaders 的完整兼容证明。

## 验收与范围

```powershell
# 9 项通过：1 共享合同、2 固定错误、3 preload/coroutine、3 file 对照。
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference

# 初始 1 项失败；完成 TODO 后 1 项通过。
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_exercises -- --ignored
```

默认 workspace 的一项 ignored 不表示练习完成。不要修改合同、固定故障、真实探针或
默认 ignored。模型只处理同步普通 Result，不模拟 loader 查找、全部 bytes 参数、
Heap failure、增量阶段或任意宿主重入；真实探针也不恢复暂停的旧审计队列。

理解题：为什么第二次返回 true 不能证明模块已修复并执行？
实现题：只完成 attempt hook，原错误返回前先清当前 key。
调试题：为什么读/编译失败保留旧 false，而 runtime error 后变成 nil？

本机证据独立在 `target/teaching-v17-2026-10-15/retry/`，为忽略产物，不随 clone 提供。
永久复现入口是本 README、contracts、固定 faults、solution 与 real 文件探针。
