# require 缓存：命中与成功提交

本包把 v0.17 当前快照的两个决定分开：`package.loaded[name]` 何时命中，
loader 成功以后怎样选择返回值、更新缓存。修改范围只有 `starter.rs` 的两个 hook；
默认参考测试有 9 项，两个 learner 测试默认 ignored，初始显式运行应各失败一项。
这不是历史版本的中间 VM，也不模拟完整 `require` 或保证全部 Lua 兼容。

从仓库根目录执行：

```powershell
$modManifest = 'book/code/v17-host-stdlib-modules/Cargo.toml'
cargo run --quiet --manifest-path $modManifest --example require_cache_problem_chain -- all
cargo test --manifest-path $modManifest --test cache_reference
cargo test --release --manifest-path $modManifest --test cache_reference
```

CLI 还接受 `lookup`、`finish`、`real`；缺省就是 `all`。它只运行固定 faults 和
solution，不读取 starter，因此能一直重放修改前后的结果。

## 问题一：有 slot 就算缓存命中吗

先预测返回 `false` 的 loader 连续 require 两次会执行几次，再运行 `lookup`。

| 设计与输入 | 第一次 / 第二次 | 最终 slot | loader 次数 |
| --- | --- | --- | --- |
| before never-cache，返回 42 | 42 / 42 | 42 | 2 |
| after truthy-cache，返回 42 | 42 / 42 | 42 | 1 |
| before existence-false，返回 false | false / false | false | 1 |
| after truthy-false，返回 false | false / false | false | 2 |

重复执行的问题不能靠“检查是否存在”解决：`false` 确实存在，但仍然 miss。
`nil` 和 `false` 才是假；`0`、空串都是真。`Cache::get` 返回拥有的 `Value` clone，
`lookup(&Cache, &[u8]) -> Option<Value>` 只有短暂共享借用，命中结果不会借用 map。
`Value` 中 table/function 等 GC handle 的 clone 不是复制对象，也不自动注册 GC root。

```diff
 let loaded = cache.get(name);
-(loaded != Value::Nil).then_some(loaded)
+loaded.is_truthy().then_some(loaded)
```

实现 TODO 1，再运行这条验收。共享合同覆盖 nil/false/true/0/空串、重复调用次数和
loader 入口看到的 `true` sentinel。TODO 2 使用参考 hook，失败不会混进本题。

```powershell
cargo test --manifest-path $modManifest --test cache_exercises cache_hit_requires_a_truthy_value -- --ignored --exact
cargo test --manifest-path $modManifest --test cache_reference cache_hit_requires_a_truthy_value -- --exact
```

## 问题二：谁拥有成功结果，首 nil 能跳过吗

`finish(&mut Cache, &[u8], Vec<Value>) -> Value` 消费 loader 拥有的返回窗口；只取
`into_iter().next()`，其余值丢弃。首值非 nil 时写缓存并返回，`false` 也属于这一支。
首值是 nil 或窗口为空时，重新读取 loader 可能修改过的 slot。

```diff
-let returned = results.into_iter().next().unwrap_or(Value::Boolean(true));
-cache.set(name, returned.clone());
-returned
+let returned = results.into_iter().next().unwrap_or(Value::Nil);
+// returned != Nil: commit returned, including false
+// returned == Nil: read current slot; only return true when slot is Nil
```

运行 `finish` 可见：忽略自行发布结果把 17 改成 true；扫描非 nil 值错误地选中第二个
返回值 99。正确版本保留发布的 17，`return nil,99` 则只处理首 nil。

| loader 动作 | 两次返回 | 最终 slot | 次数 |
| --- | --- | --- | --- |
| 不改 slot，return / return nil / return nil,99 | true / true | true | 1 |
| 发布 17，return nil 或空窗口 | 17 / 17 | 17 | 1 |
| 发布 false，return nil | false / false | false | 2 |
| 发布 17，return false,99 | false / false | false | 2 |
| 清空 slot，return nil / 空窗口 / return nil,99 | true / true | nil | 2 |

最后一行是当前快照的精确边界：**返回 true 不等于把 true 写回 slot**。
第二次 require 仍然 miss 并执行 loader。不要为了匹配某个标准库印象而悄悄改掉它。

实现 TODO 2，再显式验收。TODO 1 使用参考 hook；合同同时检查返回值、最终 slot、
执行次数与 sentinel，不以“能编译”作为成功条件。

```powershell
cargo test --manifest-path $modManifest --test cache_exercises require_success_commits_only_the_first_result -- --ignored --exact
cargo test --manifest-path $modManifest --test cache_reference require_success_commits_only_the_first_result -- --exact
```

## 真实 VM 对照与所有权边界

`model.rs` 用真实 `Value` 和 `BTreeMap<Vec<u8>, Value>` 做成功路径实验，不收集 GC。
driver 省略查找 loader 的过程，在模拟 loader 动作前放入 `true`；真实
`Vm::invoke_package` 则先取得 preload 或文件 loader，再写 sentinel，再进入禁止
yield 的调用。错误清理、循环依赖、文件搜索和重入由其他练习讨论。

`real.rs` 使用未修改的 v0.17 `Vm`，默认参考中的三个真实探针分别检查：

- 15 行精确矩阵：false/true/nil/空窗口/0/空串，loader 发布、清空 slot，以及首 nil
  后跟 99。每行返回 `[first, second, final_slot, calls]`，不是只看退出码。
- 普通返回表与自行发布表：两次结果和 `package.loaded` 中同一对象相等，实际读取
  `answer=42/43`，loader 各执行一次；发布表的 loader 返回 `nil,99`，后值不参与。
- 同一个 VM 先执行 setup，随后 host 调 `collect_garbage()`，再执行新的 chunk。
  setup 只向 host 返回 bool；局部表别名随已结束的 frame 离开，loader 不捕获模块表。
  第二个 chunk 实际读取 `answer=42` 且次数仍为 1。

第三个探针还将公开字段 `package.loaded` 替换成新表，结束旧局部 alias，再 GC。
之后 require 仍读到缓存，而新的 `package.loaded.demo` 仍为 nil。这验证的是 VM
私有初始 `package_loaded` owner。源码也保留初始 `package_preload` TableRef，
并把两者加入 roots；替换这两个公开字段不会替换 VM owner。这里没有单独实测替换
`package.preload` 字段。文件搜索的 `path` 则从当时的全局 `package` 表读取，不能
将 loaded/preload 的 owner 行为外推到 path。

```powershell
cargo test --manifest-path $modManifest --test cache_reference real_vm_cache_primitive_and_first_result_matrix -- --exact
cargo test --manifest-path $modManifest --test cache_reference real_vm_table_identity_and_loader_publication_execute_once -- --exact
cargo test --manifest-path $modManifest --test cache_reference real_vm_initial_loaded_owner_keeps_table_across_execute_and_gc -- --exact
```

完成后应能解释：为什么 false miss 而 0 hit；为什么首 nil 不寻找后值；为什么 loader
的公开 slot 写入需要在调用之后重新读；为什么拥有一个 Rust `Value` 不等于 GC 保活。
调试题：把 `scan_non_nil` 用到 finish 后，哪项字段首先偏离合同？再解释 clear-slot
场景为什么返回两次 true 却仍执行两次，不能只根据返回值推断缓存状态。
