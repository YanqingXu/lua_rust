# v0.17 · 宿主标准库与 Lua 模块

<div class="lesson-meta"><span>缓存与重试 → 文件 owner → close → GC prune → Drop drain</span><span>5-crate workspace</span><span>checkpoint v0.17</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>让 require 的加载状态与文件的宿主所有权都有可观察的边界</strong>
  <p>前置：v0.16 的 callback Result/禁止 yield、v0.13 的 Heap owner。依次解决：
  重复 require 为什么又执行；缓存为什么要看真值；A→B→A 怎样停下；首次报错为何
  让修复后的重试假成功；再区分文件已关闭、userdata 已回收、VM 已退出三个时点。
  产出是七个独立 TODO、固定反例、状态图和真实 VM/文件对照。文件部分只补所有权协议，
  os/debug API 与原有 IO 支持范围保持本 checkpoint 的边界。</p>
</div>

v0.16 的 math/string/table 函数主要处理内存中的 `Value`。文件、环境、时钟和模块加载不同：它们会接触平台状态，可能部分成功，且离开 Lua 作用域后仍需要关闭资源。本版增加一个核心能力：**让 Runtime 对宿主资源、加载缓存和调试视图拥有明确的生命周期与失败协议**。

本章把 require 拆成缓存、循环、重试，把文件拆成 close、prune、drain 六个局部实验。`faults.rs` 是人为写坏的设计，
不是历史中间 VM；CLI 固定并列错误版与参考，不读取学习者 `starter.rs`。每包另用
真实 v0.17 验证对应行为。练习模型互相独立，不拼接成另一个完整解释器。

<a id="learning-predict"></a>
## 先预测再运行

先预测下面三件事：两次 require 是否得到同一个表、B 在循环里能看到 A 的什么值、
第一次失败后修复重试会不会再运行副作用。

```lua
{{#include ../../code/v17-host-stdlib-modules/examples/require_contracts.lua}}
```

从仓库根目录运行；后面的命令继续使用同一 manifest 变量：

```powershell
$moduleManifest = 'book/code/v17-host-stdlib-modules/Cargo.toml'
cargo run --quiet --manifest-path $moduleManifest -- book/code/v17-host-stdlib-modules/examples/require_contracts.lua
cargo run --quiet --manifest-path $moduleManifest --example require_cache_problem_chain
cargo run --quiet --manifest-path $moduleManifest --example require_cycle_problem_chain
cargo run --quiet --manifest-path $moduleManifest --example require_retry_problem_chain
```

Lua 探针应返回 `type=boolean value=true`：lesson 只执行一次；循环内层看到的是 true；
retry 两次实际执行，effects 最终也是 2。CLI 退出 0 只说明固定对照重放完成，不能
代替显式 `--ignored` 的 learner 验收。接着再预测：loader 返回 false 是否只加载一次？

文件部分先预测：两个 Lua 变量指向同一个文件，关闭一个后另一个的 `type`、`io.type`
和 `read` 分别怎样变化？把两者都设为 nil，是否已经执行了宿主 File 的析构？

```lua
{{#include ../../code/v17-host-stdlib-modules/examples/file_lifecycle_contracts.lua}}
```

```powershell
cargo run --quiet --manifest-path $moduleManifest -- book/code/v17-host-stdlib-modules/examples/file_lifecycle_contracts.lua
cargo run --quiet --manifest-path $moduleManifest --example file_close_problem_chain
cargo run --quiet --manifest-path $moduleManifest --example file_prune_problem_chain
cargo run --quiet --manifest-path $moduleManifest --example file_drain_problem_chain
```

Lua 脚本应返回 true，覆盖显式 close 与别名合同；GC/Drop 的宿主释放时点另由 Rust
实验验证。脚本末尾删除文件是清理，不是证明 OS 句柄已经关闭的独立证据。

<a id="learning-code-map"></a>
## 代码阅读地图

1. [缓存实验](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/exercises/cache/README.md)：重复执行 → `Value::is_truthy` → loader 首结果与自发布；
2. [循环实验](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/exercises/cycles/README.md)：发布标记的时点 → A/B trace → 临时 true 与最终结果；
3. [重试实验](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/exercises/retry/README.md)：保存 Result → 清当前 key → 保留依赖与副作用；
4. `invoke_package::Require`：缓存读取、preload/path 定位、标记、`invoke_yield_forbidden`、提交/失败清理；
5. `compile_source → compile → Heap::allocate_function`：复用 compiler，随后在当前 VM 执行；
6. `Vm::new / gc_roots`：保存初始 loaded/preload 句柄并作为根；
7. [close 实验](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/exercises/file_close/README.md)：`invoke_io::Open → files.insert → close_file`，移动 File 与保留 tombstone；
8. [prune 实验](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/exercises/file_prune/README.md)：`gc_roots → Heap::step → finalizers → files.retain`，完成周期后对齐两种生命周期；
9. [drain 实验](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/exercises/file_drain/README.md)：`Vm::drop → files.clear → Heap::shutdown`，观察显式释放顺序与自动字段 Drop；
10. byte-oriented IO 与 debug views：内容保留 bytes，只暴露真实存在的调试数据。

这里有两套不同的生命周期：Heap 决定 Lua userdata identity 是否可达；VM map 决定宿主 File
何时关闭。它们通过 typed handle 关联，但不能互相替代。只删除 Heap record 而不 prune map，
宿主仍会持有 File，直到其他释放路径生效；只关闭 File 而不保留 tombstone，又无法实现 `io.type`。

## 文件不是普通 userdata 标签

文件值使用 `UserdataRef` 参加 Lua identity、metatable 和 GC；真正的 `std::fs::File` 保存在唯一 VM owner 的 `HashMap<UserdataRef, Option<File>>`：

Lua roots 指向 Heap userdata；VM 的 map 用同一个 typed handle 关联资源，但 **map 的 key
不是 GC root**。`Some(File)` 拥有宿主句柄，`None` 是已关闭状态，map 中没有 key 才是
“不属于这个文件 owner”。后面的三问将分别改变这三种状态。

Heap userdata 新增独立 metatable edge，collector 会追踪它。共享 file metatable 的 `__index` 指回自身，安装 read/write/close/flush/seek；因此 `file:read()` 使用 v0.12 已实现的方法调用语义，不需要在 Parser 中添加特殊语法。

资源状态机可以先写成：

```text
open:   map[id] = Some(File)
close:  take Some(File); map[id] = None
prune:  if Heap handle is stale, remove map entry
drop:   clear map before Heap shutdown
```

[查看完整宿主文件资源实现](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/crates/lua_vm/src/lib.rs)

文件有三条关闭路径：

1. `file:close()` / `io.close(file)` 将 `Some(File)` 取走并保留 closed tombstone；
2. 完整或增量 Sweep 完成后，VM 移除已经 stale 的 userdata entry；
3. `Vm::drop` 先 clear 所有文件，再执行 Heap shutdown。

closed tombstone 让 `io.type(file)` 返回 `"closed file"`，并使再次 read/write/close
产生结构化错误。删除成功不足以单独证明关闭：Unix 可 unlink 仍打开的文件，Windows
默认共享方式也可能允许删除。本章用真实 File 的 Drop 记录观察 owner 释放，并在
Windows 用 `share_mode(0)` 独占重开作补充：释放前报 sharing violation，释放后成功。
非 Windows 明示没有同等 OS 探针；不把 Drop 计数或删除当成通用内核句柄检测器。

## Byte-oriented io

`io.open` 支持 r/w/a 及其 `+` 和 `b` 组合，失败返回 `nil, message`。File methods 支持：

- `read("*a")`：读剩余全部 bytes，EOF 仍返回空 string；
- `read("*l")`：读行并移除 LF 和可选 CR；
- `read("*n")`：读取 ASCII number 并把 cursor 留在 token 后；
- `read(count)`：读取指定字节数，EOF 返回 nil；
- `write(...)`：string/number coercion 后原样写 bytes，并返回 file；
- `seek("set"|"cur"|"end", offset)`、`flush`、`close`。

测试文件包含 NUL 和 0xff，证明路径转成宿主字符串不代表文件内容也转 UTF-8。当前 Windows 教学边界要求 path 本身是 UTF-8；错误会显式返回，不做 lossy 路径匹配。

`io.read/write/flush` 连接进程 stdin/stdout。默认输入输出文件切换与 lines iterator 留在兼容收口，不用隐藏 iterator 状态假装已经完成。

## OS 边界

本版实现 clock、date、difftime、execute、getenv、remove、rename、time 和 tmpname。文件系统 API 遵循 Lua 风格：成功返回 true，平台失败返回 `nil, message`；不会把权限、文件不存在或锁冲突变成 panic。

为了不依赖宿主 locale，`os.date` 当前把 timestamp 映射为 UTC civil date，支持 `*t/!*t` 和 `%Y/%m/%d/%H/%M/%S/%c/%%`。从 Unix day 到年月日使用显式整数算法，epoch 测试固定为：

```text
1970-01-01 00:00:00, weekday=5
```

`os.execute(command)` 明确是宿主副作用边界：Windows 使用 `cmd /C`，Unix 使用 `sh -c`，返回进程状态。验收只调用无参数的 shell-presence 查询，不在测试中启动任意外部命令。

## Loader 复用同一编译器

VM 在 v0.17 依赖 `lua_compiler`，但编译器仍只依赖 core，不形成环。`loadstring`、`loadfile` 和 `dofile` 都调用同一 `compile → Proto → Heap Function` 路径：

- load 成功返回可多次调用的 Lua function；
- 语法/文件错误时 loadstring/loadfile 返回 `nil, message`；
- dofile 将加载错误作为 Lua error，并原样传播 chunk 的多返回。

这也确保动态 chunk 与主脚本共享 `_G`、registry、GC 和 coroutine scheduler，而不是启动第二个 VM。

## 问题一：同一模块 require 两次，为什么初始化又运行了一次？

### 找到 loader 只解决“怎样执行”，没有决定“是否还应执行”

让 loader 增加计数并返回一张表。忽略缓存的错误版每次都会进入 loader，计数变成 2，
两次得到的表身份也不同；文件路径正确、编译成功，都不能修复这个错误。
CLI 先重放完全不查缓存的故障，再重放“只要存在就命中”的中间设计；starter 保留
第二种故障，让读者继续发现 false 也应重新加载。
CLI 的局部模型固定返回 42，通过 calls 的 2→1 隔离重复执行问题；上面的表身份是
动机场景，另由真实 VM 的 normal/self-publication probe 检查，不能从这个数值模型推出。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example require_cache_problem_chain -- lookup
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cache/starter.rs:require_cache_lookup_starter}}
```

先检查已加载项，再决定要不要定位 loader。`Cache::get` 已把缺项转换为 Nil，检查必须
复用 Lua 的 truthiness：

```diff
  let loaded = cache.get(name);
- (loaded != Value::Nil).then_some(loaded)
+ loaded.is_truthy().then_some(loaded)
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cache/solution.rs:require_cache_lookup_solution}}
```

`lookup` 共享借用缓存，只复制返回的 Value，不复制模块表。真实 VM 通过 Heap table
保存 loaded，命中后同样复制句柄返回，所以两次访问同一 table 身份与字段内容。
nil 和 false 是 miss；0、空串、true、table 都是 hit。用 Rust 的“键存在”或数值/文本
是否为空替代 `is_truthy`，会改变重新加载的条件。

```powershell
cargo test --manifest-path $moduleManifest --test cache_reference cache_hit_requires_a_truthy_value -- --exact
```

### loader 成功了，究竟把什么写进缓存？

require 只交付 loader 的第一个结果。第二个结果不是备用值：`return nil,99` 不会
取 99。loader 也可能自己写入 loaded 再不返回值；无条件用 true 覆盖它，会丢掉它
已经发布的模块表。
starter 无条件用首值覆盖本项，空窗口则写 true：首值 nil 会删除自发布值，空窗口
又会覆盖它。这两种成功返回都需要保留 loader 的显式发布。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example require_cache_problem_chain -- finish
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cache/starter.rs:require_cache_finish_starter}}
```

协议差异是先消费首结果，再决定是否写入；不是寻找第一个非 nil：

```diff
- loader 成功就返回 true，忽略已发布值；或扫描第一个非 nil 结果
+ 首结果非 nil：写 loaded[name] 并返回它（包括 false）
+ 首结果 nil / 无结果：重读 loaded[name]，有值就返回该值
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cache/solution.rs:require_cache_finish_solution}}
```

![缓存命中、首返回值提交与 loader 自发布值的分支。](../images/v17-require-cache.svg)

| loader 行为 | 本次结果 / cache | 下次 require |
| --- | --- | --- |
| 返回 table,99 | 只提交并返回该 table | 命中同一对象 |
| 返回 false | 提交 false，返回 false | miss，再执行 |
| 返回 0 或空串 | 提交对应值 | 命中 |
| 不返回 / 首值 nil，未改 slot | 调用前 true 标记仍在 | 命中 true |
| 自写模块表，首值 nil | 保留并返回自发布表 | 命中同一对象 |
| 清空本项，再返回 nil | **仅返回 true，slot 仍是 nil** | miss，再执行 |

最后一行是这个 checkpoint 的精确边界：真实源码没有在 nil fallback 时把 true
写回。模型与真实探针都保留这一行为，不能悄悄修成“任何 nil 返回都缓存 true”，再
声称讲解的是现有源码；本章也不宣称与所有 Lua 版本的 require 规则相同。

**完成条件：** 两个 hook 分别通过命中与提交合同；真实 VM 检查 loader 次数、表身份、
原始值、首 nil/后值和自发布行为。跨 execute 的 GC 检查要求实际读取缓存对象字段，
不能只比较句柄显示文本或 map 长度。精确命令见缓存包 README。

```powershell
cargo test --manifest-path $moduleManifest --test cache_reference require_success_commits_only_the_first_result -- --exact
```

这个真实探针特意替换公开的 `package.loaded` 字段并结束执行帧，再从宿主触发 GC。
随后 require 仍命中 VM 保存的初始 owner，实际读取 answer=42；新公开表里没有这项。
它证明字段重绑与内部 owner 不同，不把 Rust handle 的保存本身说成保活。

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cache/real.rs:require_cache_real_owner}}
```

缓存现在能挡住重复执行。但第一次执行还没返回时，另一模块已经回来 require 它了。

## 问题二：A 要 B，B 又要 A，第一次加载应该何时变得可见？

### 在 loader 返回后才发布，阻挡不了重入

错误版把“完成后提交”当成唯一发布时点。A 的 loader 进入 B，B 又进入 A，因为双方
都还没返回，cache 一直未命中，产生 A→B→A→B 的重复进入。实验用固定有限预算让
错误版停下，避免用真实 stack overflow 教学；这个预算不是生产循环协议。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example require_cycle_problem_chain
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cycles/starter.rs:cycle_publish_starter}}
```

最小改变是把加载标记的写入移到调用前。此时要发布的是 true，还不是最终模块表：

```diff
+ vm.publish(name, Value::Boolean(true))?;
  let returned = loader(vm, name)?;
- vm.publish(name, Value::Boolean(true))?;
  vm.publish(name, returned.clone())?;
  Ok(returned)
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/cycles/solution.rs:cycle_publish_solution}}
```

![A/B 加载前先发布 true，B 重入 A 时命中标记，最终分别提交 B/A 的结果。](../images/v17-require-cycle.svg)

真实顺序是：缓存 miss → 定位 loader → 写 true → `loader(name)`。A→B→A 的内层
require(A) 得到 Boolean(true)，不会重新运行 A；B 返回表后提交 B，A 随后再提交 A。
这能终止固定循环，却不能让 B 在此时访问尚未完成的 `A.answer`。

### true 代表“完成”还是“进行中”？

本快照没有独立 `Loading` enum，也没有不能伪造的内部 sentinel。加载标记和模块
最终返回的 true 走同一个 truthy 分支，因此单看值不能判断初始化阶段。若 loader
自行提前发布 table，内层可以读到它，但那是 loader 明确建立的边，不是 require
自动创建完整模块对象。

**完成条件：** 模型自循环/A-B 循环的 trace 与次数正确；真实 preload 和 Lua 文件
分别验证内层看到 true、最终缓存是实际表。固定预算耗尽只证明晚发布的反例会重复
进入，不应被当作真实 VM 的循环错误类型。本包循环成功路径的模型不承担失败恢复，
只处理固定单结果 loader；nil/false 提交矩阵由缓存包负责，下一问再单独检查失败恢复。

```powershell
cargo test --manifest-path $moduleManifest --test cycles_reference loading_sentinel_is_visible_before_recursive_loader_call -- --exact
```

## 问题三：第一次 loader 报错，为什么修好之后却得到 true？

### `?` 把普通错误交回了，却把加载标记留下了

先让 loader 增加副作用计数，再返回 Err；修复 loader 后用同一个 VM 再 require。
错误版若直接 `loader(...)?`，会跳过删除，第二次命中残留 true，不再运行修复后的
loader。看起来“没有再报错”，实际上连修复代码也没执行。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example require_retry_problem_chain
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/retry/starter.rs:retry_attempt_starter}}
```

把 Result 留在局部变量中，明确匹配 Err，再决定删除与返回的顺序：

```diff
- let results = loader(vm, name)?;
- Ok(vm.finish(name, results))
+ let result = loader(vm, name);
+ match result {
+     Ok(results) => Ok(vm.finish(name, results)),
+     Err(error) => { vm.loaded.remove(name); Err(error) }
+ }
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/retry/solution.rs:retry_attempt_solution}}
```

模型保留原 VmError 验证种类、位置和消息，清理只删除当前 key。真实 VM 使用
`Heap::table_set(name, Nil)` 删除本项；若这个 Heap 清理本身失败，源码会用 cleanup
错误替换原错误，所以“总能返回原错误”只适用于删除成功的普通路径。

![发布前错误与发布后错误的差别；只删当前模块，保留依赖缓存与已发生副作用。](../images/v17-require-retry.svg)

### 为什么不能直接清空整个 package.loaded？

A 加载 B 成功后，A 自己报错。清空整个 loaded 会连 B 和原有 prefix 模块都删掉，
导致重试 A 时 B 被无故再执行。正确清理只令 A=nil，B 继续命中；A 的副作用则已经
发生，第二次运行会再次发生。这是当前缓存项的失败清理，**不是回滚整个依赖图或
Lua 全局副作用**。

它也不是“恢复进入前的旧值”：原项为 false 时，运行阶段失败后变 nil；loader 先
自发布 table 又报错，当前项同样删掉。这个固定合同要与文件定位失败区分。

### 所有失败都发生在标记写入之后吗？

| 阶段 | 发生的错误 | 当前 loaded 项 |
| --- | --- | --- |
| 查缓存后、定位 loader 时 | 缺文件、package.path 无效 | 尚未写新标记，原值不因本阶段失败被删 |
| 读到首个文件后编译 | 语法错误 | 同样发生在标记之前；不会继续试后一个模板 |
| 写 true 后执行 loader | 普通运行错误、不可调用 preload、禁止 yield | 删除当前 key，再返回错误 |
| 正常提交/清理时 | Heap API 本身失败 | 遵循实际错误分支，模型不注入分配/清理失败 |

文件 probe 使用同一 VM 和独立临时目录，先分别复现 missing/compile/runtime，再
修复文件后重试；preload probe 则在同一次 execute 内替换 loader 并重试。另一个
coroutine 在 require 拒绝 loader yield 后继续运行，修好 loader 后加载成功，再在
同一 coroutine 合法 yield/resume，避免用新 VM 的初始化掩盖禁止状态泄漏。

**完成条件：** 不只要求第二次不报错，还要检查新 loader 的次数/结果、旧错误的保留、
当前项精确 nil、依赖 B 与 prefix 保留，以及副作用没有被伪造回滚。这个 Result
模型不承诺 panic/abort、任意 callback 资源或全部 GC 阶段都能恢复。

```powershell
cargo test --manifest-path $moduleManifest --test retry_reference failure_removes_only_the_current_loading_entry -- --exact
```

## 从真实 require 源码读出边界

[完整 module loader](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v17-host-stdlib-modules/crates/lua_vm/src/lib.rs)
从 `invoke_package::Require` 开始。不要一次 include 全函数；依次沿三个练习中的局部
合同回到源码的 lookup、publication、invoke 和 cleanup/commit 分支。

- 先查初始 loaded 表；preload 非 nil 就按 callable 路径尝试，false 也不是“没有 preload”。
- 文件搜索把模块名的 `.` 换成 `/`，把每个分号模板中的 `?` 替换为该路径；空模板忽略。
  读取失败累积候选诊断，首个可读文件编译失败则立即返回，不继续搜索。
- 当前 compiler 产生 Proto 并分配 Heap Function，loader 在同一 VM/global/registry
  执行，参数是模块名字；不是再启动一个隔离 VM。
- VM 保存初始 `package_loaded/package_preload` 句柄并直接枚举为 roots；只替换
  `package.loaded` 或 `package.preload` 字段不会换掉这两个内部 owner。文件 path
  则从当前 global `package` 表读取。模型的缓存容器不证明这些 Heap/GC 行为，须看 real 对照。
- 从 loader 结果返回到缓存提交的现有同步路径没有新增 GC checkpoint。不能因为局部
  Vec 拥有 Value 就说任意时点都安全；v0.15/v0.16 已记录的 GC 缺口不由本章覆盖。

`package.loadlib`、原生动态模块及完整 package searcher 协议不在本 checkpoint 内。

## 问题四：文件已经 close，为什么 userdata 还应存在？

### 先画出谁真正拥有 File

`io.open` 先由 Rust 局部变量拥有 `File`，成功分配 userdata 并安装 metatable 后，
才将它移动进 `files[id] = Some(file)`。若此前的 Heap 操作返回 Err，局部 File 会随
返回析构。Lua 值只携带 `UserdataRef`；复制给 alias 只复制身份，不复制 File 或 OS
句柄。方法调用短暂借用 map 中的 File，不能从这次借用获得新的长期 owner。

![Lua 可达性、userdata 身份与宿主 File 所有权，以及 close 保留 tombstone 的迁移。](../images/v17-file-ownership.svg)

现在预测两个错误设计：只写 closed flag、仍保留 `Some(File)`；或者直接删除整个
map entry。前者声称 `claimed=true`，但 `io.type` 仍读到 open，File 仍由 map 拥有；后者确实会析构 File，
但 `io.type(alias)` 变成 nil，重复 close 也从“已关闭文件”变成“不属于文件 owner”。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example file_close_problem_chain -- close
cargo run --quiet --manifest-path $moduleManifest --example file_close_problem_chain -- identity
```

### `take` 改变 owner，tombstone 保留协议身份

实现题只改 `close(vm, handle)`。输入是 typed identity 与对 VM 的短暂可变借用；
要同时完成释放和状态迁移，不能只让一个 Boolean 变真。

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/file_close/starter.rs:file_close_starter}}
```

```diff
- files.remove(&handle)  // File 会 drop，但已关闭身份也丢失
+ let slot = files.get_mut(&handle).ok_or(NotFile)?;
+ let file = slot.take().ok_or(Closed)?; // map 留下 None
+ drop(file);                           // 移出的 owner 在此释放
```

这是协议差异示意，错误类型和完整接口以参考代码为准：

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/file_close/solution.rs:file_close_solution}}
```

`Option::take` 把值移动出来并留下 None；**不是调用 take 就必然关闭**。若把取出的
File 存入另一个 owner，句柄还会继续存在。参考把它立即 drop；真实 `close_file`
在 `file.take().is_none()` 条件表达式结束时销毁这个临时 Option，因此成功返回前
已释放 File。`None` 仍是 map 中的条目，Heap userdata、metatable 与 Lua alias 都不变。

| 输入身份 / 状态 | `io.type` | close | read/write/seek/flush |
| --- | --- | --- | --- |
| 已登记且 Some(File) | `"file"` | 释放 File，留下 None | 借用同一 File 执行 |
| 已登记且 None | `"closed file"` | Runtime：重复关闭 | Runtime：使用已关闭文件 |
| 不属于 files 的 userdata | nil | Type：not a file handle | Type：not a file handle |

这不是重复 close 也成功的幂等 API。`type(alias)` 仍是 userdata，`io.type(alias)`
却成为 closed file；两个观察回答不同的问题。真实 VM 对照先用 `io.close(f)`，再经
另一个别名调用 `:close()`，并在 GC 后读 metatable/别名状态，避免只检查模型 flag。

```powershell
cargo test --manifest-path $moduleManifest --test file_close_reference close_releases_owner_but_keeps_closed_identity -- --exact
```

**完成条件：** File 的 Drop 记录增加一次，reachable userdata 与 None 条目仍在，
别名语义和错误分类一致；Windows 探针先证明同一文件仍被占用，再证明独占重开成功。
仅有 `io.type == "closed file"`、map 长度变化或 CLI 退出 0 都不够。

## 问题五：Lua 已经不再引用文件，宿主 map 为什么还占着句柄？

### GC 只认识 Heap graph，不会替另一个容器执行 retain

把局部变量设成 nil，只是移除一条 Lua edge。真正回收还要等 collector 运行；即使
Heap 已经删除 userdata，`HashMap<UserdataRef, Option<File>>` 仍可能保留原条目。
typed handle 在这里是可复制的标识，不会自动成为强引用，也不会自动同步两个容器。
错误版完全不 prune，可以观测到 **Heap 已 stale、File owner 仍在** 的间隙。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example file_prune_problem_chain
```

![GC 完成后的四格清理：保留两个 live 状态，移除两个 stale 状态；map key 不作 roots。](../images/v17-file-prune.svg)

练习用真实 Heap 和实际 File，先观察 Sweep 已结束而宿主 prune 尚未运行，再执行一段
很小的清理。实现题只补 `prune`，不会改 collector 或 IO 实现：

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/file_prune/starter.rs:file_prune_starter}}
```

```diff
- // 只推进 Heap，不对齐 files
+ if step.completed.is_some() {
+     files.retain(|id, _| matches!(heap.is_live(*id), Ok(true)));
+ }
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/file_prune/solution.rs:file_prune_solution}}
```

`retain` 在原 map 上逐项判定；删除 Some 条目时，File 随 value 析构。判据是该 typed
userdata 在 Heap 中仍存在，而不是 slot 是否 Some。四格规格如下：

| GC 完成时的 Heap 状态 | files slot | prune 后 | 可观察结果 |
| --- | --- | --- | --- |
| live | Some(File) | 保留 Some | 文件仍可使用 |
| live | None | 保留 None | 仍能辨认 closed file |
| stale | Some(File) | 删除条目并 drop File | 不可达文件的宿主 owner 被释放 |
| stale | None | 删除 tombstone | 不再保留无用途的登记项，不重复关闭 |

### 为什么既不能保留所有 key，也不能只保留 Some？

把 files 的所有 key 加进 `gc_roots`，会让这些 userdata 因 map 自己而永久可达，
prune 永远等不到 stale。反过来按 `Option::is_some` retain，会删掉仍可达的 closed
身份，同时保留已经 stale 的 open owner。两种错误都可能让 map 看起来“更整齐”，
却破坏上述四格合同；固定 CLI 分别重放它们。

真实 `gc_step_internal` 的顺序是 `Heap::step` → 取出并保活 finalizer events → 调
callback → 若 `step.completed` 则 prune → 释放临时 roots → 返回首个 callback 错误。
所以不应把任意一次 budget 耗尽当作整个周期完成。`is_live` 检查对象记录是否存在，
白对象在 Sweep 前仍可能返回 true；本章不制造“提前 retain 必然立刻误删”的结论，
而是按当前源码的完成周期边界提交清理。

带 finalizer 的 userdata 也可能在本周期被保留，并因 resurrection 延续可达性。
“变量离开作用域”“完成一次 GC”“宿主 File 已 drop”不能画等号。参考四格不包含
任意 finalizer 组合；真实对照用普通文件、多个 execute 与宿主 GC，先结束旧执行帧，
再检查弱槽精确为 nil 和 Windows 独占重开，避免隐藏的帧值保活掩盖问题。

```powershell
cargo test --manifest-path $moduleManifest --test file_prune_reference completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones -- --exact
```

**完成条件：** 完成前不做协议清理，完成后四格结果与文件 owner 状态准确；live None
仍可查询，stale Some 真正释放，所有 key 不因宿主登记被重新加入 roots。

## 问题六：VM 退出时还全局可达的文件，要等下一次 GC 吗？

### Drop drain 是 owner 结束，不是再运行一次 Lua 回收

脚本可以把打开的文件留在全局表；只要 VM 还活着，这就是正确的 reachable open。
但 VM owner 结束后，不能依赖再一次 Lua 调用来关闭它。真实 `Vm::drop` 明确先清空
文件 map，再 shutdown Heap：打开项析构 File，None tombstone 直接移除。

```powershell
cargo run --quiet --manifest-path $moduleManifest --example file_drain_problem_chain -- before
cargo run --quiet --manifest-path $moduleManifest --example file_drain_problem_chain -- after
```

![Vm Drop 先 drain 文件再 shutdown Heap；错误顺序的字段仍会在 Drop body 后自动析构。](../images/v17-file-drain.svg)

先预测固定故障：自定义 Drop body 只调用 `heap.shutdown()`，会永久泄漏所有 File 吗？
**普通 Rust 字段仍会在自定义 Drop body 结束后自动析构。** 因而这个故障的可验证差异
是 shutdown 观察点仍持有 File，不是整个 owner 销毁后必然泄漏。实验记录释放次数
与事件顺序，在两种路径完全结束后都要求资源释放。

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/file_drain/starter.rs:file_drain_starter}}
```

```diff
  fn drop(&mut self) {
+     self.files.clear(); // 先结束宿主资源所有权
      self.heap.shutdown();
  }
  // 自定义 Drop body 结束后，Rust 仍会继续析构字段
```

```rust
{{#include ../../code/v17-host-stdlib-modules/exercises/file_drain/solution.rs:file_drain_solution}}
```

实现题只补 shutdown hook 的 drain 顺序。`clear` 对全部 entries 生效，不看 Lua 是否
仍可达；两个别名不产生两次 File 释放，已关闭的 None 也不产生额外关闭。Heap 的
`shutdown` 清空 objects、collector 队列和 ready finalizers，并标记 shutdown；它不
等价于完整 collect，不会执行 Lua 或补跑 pending finalizer。

```powershell
cargo test --manifest-path $moduleManifest --test file_drain_reference solution_drains_some_and_none_before_heap_shutdown -- --exact
```

**完成条件：** 正确版的事件记录显示所有 Some 已在 Heap shutdown 前释放；错误版
准确显示 body 内仍持有、字段自动 Drop 后归零。Windows 真实 Vm 的全局文件在 owner
作用域内阻挡独占重开，离开作用域后可独占重开并实读内容。这个 Drop 合同不返回关闭错误，
不保证 flush/fsync 持久化，也不延伸为 process exit/abort 的清理承诺。

### 三问合并后的最小迁移路线

1. 保持 userdata、metatable 与 methods 不动，先用 `take + None` 补齐显式 close；
   以 aliases、重复操作分类和 File 释放验收。
2. 保持 files 不作为 roots，在完成周期后按 `is_live` retain；用四格和真实跨 execute
   probe 验收，不能以“对象不再有局部变量”代替完成 GC。
3. 在 VM owner 退出路径先 clear files 再 shutdown Heap；用 body 内的顺序观察点和
   owner 完全销毁后的真实重开分别验收。

这三个局部模型不拼成另一份 VM；每个只引入一个 TODO，并用固定错误版、参考、真实
快照三层证据说明哪些关系是模型合同，哪些已经由现有实现兑现。

## Debug 视图

debug 库公开本阶段能由真实 Runtime 数据支撑的入口：getinfo、getmetatable、setmetatable、getregistry、getupvalue、setupvalue 和 traceback。

- getinfo 接受 Lua/native function 或活动 stack level，读取 Proto debug name、upvalue 数和 line；
- getupvalue/setupvalue 直接访问 Closure 的 typed upvalue handles，并返回编译器记录的名字；
- debug metatable API 绕过 `__metatable` 保护，覆盖 table、userdata 和 primitive type；
- registry 返回 v0.13 已作为 GC root 的真实 registry table；
- traceback 基于当前 call-info 生成 byte string。

当前还没有 lexical local name/location 表、hook event 或 function environment，所以相应 debug API 不注册，而不是返回猜测值。

## Rust 知识映射与设计选择

- `Option<File>` 同时表达 open resource 与可观察的 closed userdata；
- file metatable 是 Heap graph edge，宿主资源 map 是 Runtime ownership edge，两者职责不混淆；
- `OpenOptions` 把 Lua mode string 翻译为显式权限组合；
- `Result<Vec<Value>, VmError>` 表达 Lua error，`Vec<Value>{nil,message}` 表达普通平台失败；
- `loaded[name] = true` 在 loader 调用前发布；调用后的普通错误只删当前 key，定位失败时还没有新标记；
- loader callback 在禁止 yield 的宿主边界执行，避免暂停一半的 require transaction；
- VM Drop 先 drain 宿主资源，再 shutdown Heap；字段随后仍按 Rust 规则自动析构。

替代方案是把 `File` 直接塞进 Heap object，但会迫使 core 依赖 `std::fs`，破坏 runtime value 与宿主能力的边界。另一个方案是全局 integer fd；typed userdata identity 更自然地复用 metatable、GC 和错误分类。

## 从 v0.16 迁移了什么

- 新增：io/file、os、package、debug builtin catalog；
- 新增：Userdata 独立 metatable 与 GC tracing edge；
- 新增：open file owner map、三条资源 drain 路径和二进制 IO；
- 新增：UTC time、环境、文件系统、shell status 与平台错误协议；
- 新增：loadstring/loadfile/dofile 共用编译器；
- 新增：package.path/preload/loaded、Lua file require、循环 sentinel、缓存与 rollback；
- 新增：Proto/upvalue/registry/metatable/traceback debug 视图；
- 保留：v0.16 核心库矩阵与 v0.15 coroutine/GC 合同；
- 排除：动态 native module、C API 和 binary chunk。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml -- `
  book/code/v17-host-stdlib-modules/examples/demo.lua
cargo test --manifest-path $moduleManifest --test cache_reference
cargo test --manifest-path $moduleManifest --test cycles_reference
cargo test --manifest-path $moduleManifest --test retry_reference
cargo test --manifest-path $moduleManifest --test file_close_reference
cargo test --manifest-path $moduleManifest --test file_prune_reference
cargo test --manifest-path $moduleManifest --test file_drain_reference
cargo fmt --manifest-path $moduleManifest --all -- --check
cargo clippy --manifest-path $moduleManifest --workspace --all-targets -- -D warnings
cargo test --manifest-path $moduleManifest --workspace
cargo test --manifest-path $moduleManifest --workspace --release
```

示例 require 一个只加载一次的 Lua module，写入并重读临时文件，再用 debug.getinfo 检查 module function：

```text
type=number value=1717
```

原快照基线有 119 项：74 个主线端到端、7 个核心标准库/差分、10 个宿主标准库与模块、
7 个编译器、15 个 core、3 个 stdlib 合同与 3 个 VM。require 包增加 24 项参考
（cache 9、cycles 6、retry 9）；文件包增加 18 项（close 8、prune 6、drain 4），
合计 **161 通过、7 个 learner ignored**。不把 ignored 当作完成；文件探针使用唯一
临时目录、核对路径范围并显式清理，不改变工作目录。

```powershell
cargo test --manifest-path $moduleManifest --test cache_exercises -- --ignored
cargo test --manifest-path $moduleManifest --test cycles_exercises -- --ignored
cargo test --manifest-path $moduleManifest --test retry_exercises -- --ignored
cargo test --manifest-path $moduleManifest --test file_close_exercises -- --ignored
cargo test --manifest-path $moduleManifest --test file_prune_exercises -- --ignored
cargo test --manifest-path $moduleManifest --test file_drain_exercises -- --ignored
```

初始六组分别有 2/1/1/1/1/1 项失败。只修改各包 `starter.rs` 指定 hook，完成后同一合同通过。
各题可独立完成；固定 CLI 和默认参考不读取 starter，不需要先完成上一包。
维护者需验证“初始失败 → 临时套解通过 → 原字节恢复后再失败”，防止假绿或交付已填答案。
逐题命令、成功观察和模型范围见各包 README。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/check_book_teaching.ps1
mdbook build book
node tools/check_book_links.mjs target/mdbook .
```

## 本版本暂不支持

- io.input/output 默认流切换、io.lines/file:lines、io.tmpfile 和 setvbuf；stdin 的 `*n` 尚未实现；
- os.exit、setlocale，以及 os.time(table)；os.date 不是宿主 localtime/locale 的完整 formatter；
- package.loaders/cpath/loadlib、`module(...)`/package.seeall 和动态 native module；
- debug.getlocal/setlocal、gethook/sethook、debug.debug、getfenv/setfenv；
- loader 的 chunk name 仍是教学 Proto name，traceback 尚未携带规范化 `@path`；
- Windows host path 当前要求 UTF-8，未覆盖任意宽字符路径；
- v0.16 尚未实现的 Lua pattern 与完整 string.format 仍未补齐。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题：同一个 true 为什么可能处在不同阶段？</strong>
  <p>沿循环图说明发布标记、重入命中、最终提交的先后；再沿缓存表说明 nil、false、0
  和空串分别何时再次执行。运行 cache/cycles CLI 对照；成功条件是能解释循环中拿到
  true 为什么不能读取完整模块字段，以及它为何不等于已完成初始化。</p>
</div>

<div class="exercise implementation">
  <strong>实现题：完成七个独立 hook</strong>
  <p>只修改 cache 的 lookup/finish、cycles 的 publication、retry 的 attempt。
  文件部分再补 close 的 take、prune 的完成周期 retain、drain 的 shutdown 顺序。
  运行上面六个 --ignored 命令；2/1/1/1/1/1 项全部通过才完成。不要改固定故障、合同、
  真实核心或默认 ignored 标记来取得通过。</p>
</div>

<div class="exercise debugging">
  <strong>调试题：重试不报错，是否真的执行了修复代码？</strong>
  <p>运行 retry CLI，对照漏删当前项与清空全部缓存两种故障。检查第二次 loader 的次数
  和结果、B 的执行次数、当前 key 与 prefix；成功条件是能解释“假命中 true”和
  “无故重跑成功依赖”的不同原因，并说明副作用为何仍存在。</p>
</div>

<div class="exercise understanding">
  <strong>理解题：closed、stale 和 owner 结束能否互换？</strong>
  <p>沿三幅文件状态图说明 f 与 alias、Heap userdata、map key、Some(File) 分别由谁
  保活。用 close/prune/drain CLI 核对四格与事件顺序；成功条件是能解释 live None
  为什么必须保留，以及为何 map 的 key 不能反过来成为全部文件的 GC roots。</p>
</div>

<div class="exercise debugging">
  <strong>调试题：删除成功、最终归零，是否已经证明顺序正确？</strong>
  <p>只读固定 faults，对比 flag-only、remove、漏 prune、按 Some prune、只 shutdown。
  先列每项错误会在哪个观察点暴露，再运行对应 CLI；成功条件是能分清 Lua 语义、
  Rust owner Drop 和 Windows 独占重开三层证据，并解释错误 drain 为何仍会在字段
  自动析构后释放，而不能据此声称其 shutdown 前的顺序正确。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出 userdata、VM resource map 和 OS handle 的 owner 图；
- 能解释 take 移动 owner、None 保留身份和 not-file/closed-file 的错误分类；
- 能按四格验收 completed 周期后的 prune，并说明 finalizer/resurrection 的保留边界；
- 能解释 shutdown 前 drain 与 Drop body 后字段析构的区别，不把删除成功当关闭证明；
- 能用真实值与执行次数解释缓存命中和首结果提交，而不是只看 key 是否存在；
- 能手推 A→B→A 的缓存轨迹，区分循环中的 true 与最终模块表；
- 能区分标记发布前后的错误，并验证当前项清理、依赖保留与同 VM 重试；
- 七个独立 TODO 显式通过，能分清模型、真实快照、GC/禁止 yield 与 OS 探针的验证范围；
- 能区分 Lua error 与 `nil,message` 平台失败。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 scoped runtime access、HostIo、package searcher 与 debug hooks 是本版显式 owner map 的扩展目标；
- [Runtime 所有权与 GC](v13-runtime-gc.md) 可回看为什么 registry/package/file metatable 必须进入 root inventory；
- [Coroutine](v15-coroutine.md) 可对照 require/file callback 为什么属于禁止 yield 的宿主边界；
- [核心标准库](v16-core-stdlib.md) 可回看 catalog 和 HostError 如何扩展到平台失败；
- 本版六组问题链已覆盖 require 与文件 owner；若继续补 IO，可从“读到空字节串、EOF 与读取失败如何区分”推进 bytes/Result 协议，独立于本章资源生命周期验收。
- 下一 checkpoint 是 v0.18：完成 CLI/REPL 合同，并新增 `lua_bytecode` crate 提供 text/JSON、CFG、trace、fixture manifest 与超时 runner。
