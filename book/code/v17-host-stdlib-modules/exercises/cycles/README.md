# v0.17：A 等 B，B 又 require A，什么时候发布加载哨兵？

先预测：A 的 loader 调用 `require('B')`，B 的 loader 又调用 `require('A')`。如果直到
loader 返回才写入 `package.loaded`，内层调用能看见 A 已经开始加载吗？

本包固定本快照的协议：**loaded 中任何 truthy 值先命中；调用 loader 前先放 true；
正常 loader 返回后再写入最终 exports。** 不是引入一个私有 Loading enum，也没有把
循环改成自动报错，更没有默认替 loader 创建 partial table。

## 先用有界反例观察循环

从仓库根目录运行：

```powershell
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_cycle_problem_chain -- cycles
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_cycle_problem_chain -- truthy
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_cycle_problem_chain -- real
```

省略参数或使用 `all` 可重放全部。CLI 固定使用 `faults.rs` 和 `solution.rs`，不读取
starter。退出码 0 只表示错误/修复对照跑完，不表示学习者完成了 TODO。

模型用真实 Heap 保存 loaded 和 exports，loader 是预置 Rust 函数。预算 6 只限制
**loader 进入次数**，是避免故障实验无限递归的保护；它不是 require 的 Lua 语义，
也不是通过增加真实 VM 的调用深度来实现循环检测。

| 输入 | 太晚发布 true | 调用 loader 前发布 true |
| --- | --- | --- |
| self → self | loader 进入 6 次，实验返回 `Err(Budget)` | self loader 只运行 1 次，内层得到 Boolean(true) |
| A → B → A | A/B 各进入 3 次，实验返回 `Err(Budget)` | A/B 各运行 1 次，B 内层得到 Boolean(true) |

循环成功后的返回值要分层看：

```text
require(A) miss → loaded[A]=true → 进入 A
  require(B) miss → loaded[B]=true → 进入 B
    require(A) hit → 返回 true
  B 返回最终表 { name='B', answer=7, inner=true } → 覆盖 loaded[B]
A 返回最终表 { name='A', answer=42, inner=B表 } → 覆盖 loaded[A]
```

B 存下来的 `inner` **仍是 Boolean(true)**，不会在 A 完成后自动变成 A 表。最外层
则必须返回可实际读字段的 A/B 表，后续 require 命中同一最终对象，不能把“看见 true”
当成“模块字段已经初始化完成”。

自循环的精确轨迹为：

```text
enter self
publish self: boolean
cache self: boolean
publish self: table
cache self: table
```

最后一项是外层完成后的第二次 require，需与最终 exports 身份相同。CLI 会打印
模块 name、answer、inner 和完整 trace；合同另实际读取字段并核对调用次数。

## 唯一 TODO：把 true 发布到 loader 调用之前

只修改 `starter.rs::run_loader`。入口已在 `require` 中先按 truthiness 查缓存；这里只
移动发布的时间点，保留正常返回后提交最终 exports 的步骤：

```diff
- let returned = loader(vm, name)?;
  vm.publish(name, Value::Boolean(true))?;
+ let returned = loader(vm, name)?;
  vm.publish(name, returned.clone())?;
  Ok(returned)
```

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_exercises loading_sentinel_is_visible_before_recursive_loader_call -- --ignored --exact
```

初始执行 **1 项且失败**：self 循环耗尽实验预算。修复后同一项通过，继续验证自循环
和 A/B 循环的进入次数、内层 true、最终表身份与实际 name/answer/inner 字段。
不要增大预算、禁止所有重入、提前缓存一个任意 table，或只在最终返回值中伪造 true。

`solution.rs` 是可选参考。默认 workspace 会把这一项列为 ignored；默认通过或 CLI
成功退出不能代替显式 TODO 验收。合同与固定 fault 不应随学习者修改而改变。

## true 缓存命中为什么不能表达“初始化已完成”？

真实 `invoke_package` 的开头只调用 `loaded.is_truthy()`，没有额外的加载阶段标记：

- 加载中的 true 与一个已经正常 `return true` 的模块，在 cache 中都是同一个 Value
  形态；本版无法仅靠这个值区分二者。
- loader 可主动写入自己的 partial table，再重入 require。内层会直接得到这张 truthy
  表，即使 `ready=false`；这是 loader 显式发布的结果，不是 require 自动保证初始化。

模型和真实 preload 都有这个对照：loader 发布 `{ready=false}`，重入读到同一表且
仍未 ready，随后设置 ready=true 并返回。另一个 loader 正常返回 true，重复 require
只进入一次。对照只证明此 loaded 协议，不能外推所有模块初始化次序安全，也不把它
作为官方 Lua 5.1 循环行为的兼容证明。

## 真实 VM：preload 与文件加载分别验证

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_reference
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_reference real_vm_preload_self_and_mutual_cycles_see_true_then_tables -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_reference real_vm_explicit_partial_exports_and_completed_true_use_the_same_cache -- --exact
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_reference real_vm_file_modules_commit_final_exports_after_cycle_sentinel -- --exact
```

六项默认参考包括：成功合同、固定晚发布故障、模型 truthy/partial 对照，以及上述
三项真实 VM 对照。真实探针不使用模型 loaded 作为 VM 输入，而是分别通过真实
`package.preload` 和实际临时 `.lua` 文件触发现有 VM 的 require。

preload 与文件两套都核对 self/A/B 的真实 exports 字段、缓存身份和 loader 只运行
一次，内层 A 必须是 true。文件对照随后结束执行帧、host GC，再从真实 package.loaded
读取 A→B 身份以及 42/7，避免只检查一个 cache 布尔值就宣告初始化完成。

临时文件使用进程号、纳秒时间与原子序号的唯一目录，不修改工作目录或全局环境。
清理前 canonicalize 目标并确认它仍是本任务临时根下的独立子目录；正常路径显式
`finish`，同时出现执行错误和清理错误时保留两者，执行错误在前。提前 unwind 另有
Drop 兜底，清理失败只报告，不用新的 panic 覆盖原错误。临时目录不作为教学交付物。

## 模型边界

预置 loader 正常返回 table 或 true；本题只教缓存重入和发布时间点，不实现文件搜索、
缺失模块诊断、nil/false 提交策略、loader 失败或嵌套依赖回滚。尤其不能把这段只有
正常提交的 `?` 示例当作完整 require：真实 VM 的 Err 分支还需要清理当前 loaded，
该问题另有教学包。预算错误只是故障实验出口，不代表真实错误恢复合同。

真实 VM 的模块 cache、preload 和路径句柄各有自身 owner/根协议；本模型没有在任意
中间点执行 GC，不据此证明所有 loader callback 或 GC 交接路径安全。
永久复现入口是本 README 与 `model.rs`、`contracts.rs`、`real.rs`、`reference.rs`。
