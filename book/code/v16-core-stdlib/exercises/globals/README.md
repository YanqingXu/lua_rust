# v0.16：裸全局与 `_G` 为什么不能维护两份存储？

先预测：把 Rust `HashMap` 的初始内容复制到 `_G` 表后，`forward = 12` 会自动改变
`_G.forward` 吗？反过来 `_G.reverse = 34` 会改变裸全局 `reverse` 吗？两边分别赋 nil
时，删除会不会传播到另一份副本？

本包用真实 `Heap` 构造两种环境路由，固定相同输入观察差异。它不是历史中间 VM，
也没有向真实 VM 注入双存储故障。唯一 TODO 实际分配环境、初始化 seed 与 `_G` 自指；
全局/字段路由、根枚举与验收合同都已给出。

## 问题从哪里来？

错误设计同时维护：

```text
bare global get/set → Rust HashMap<ByteString, Value>
_G.field get/set    → 初始复制出来的独立 Heap table
```

复制只是一个时间点的数据一致，不会建立后续同步协议。即使 `_G._G` 最初指回该快照
表，两个写入目标仍是两个 owner。修复是让两个入口在初始状态使用**同一 Heap 表**：

```text
Vm.global_env ──直接 GC root──> Heap environment table
GETGLOBAL / SETGLOBAL ────────> 同一 environment table
environment['_G'] ────────────> 初始也指回这张表
```

真实 v0.16 的 `Vm::new` 把 builtin/catalog/library 字段装进 `global_env`，之后再安装
`_G` 自指。`GETGLOBAL` 与 `SETGLOBAL` 始终对私有 `Vm.global_env` 调用 Heap API；
`gc_roots` 直接加入 `Value::Table(global_env)`，不是每次从 `_G` 变量寻找环境。

Rust 的 `TableRef` 是可复制 handle。复制它不会复制 table，也不会自动注册 GC root。
即使 `env['_G']=env` 构成自环，没有外部 root 时环境及其子图仍可回收。

## 先跑固定前后对照

从仓库根目录运行：

```powershell
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example globals_problem_chain -- coherence
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example globals_problem_chain -- identity
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example globals_problem_chain -- gc
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml --example globals_problem_chain -- real
```

省略参数或使用 `all` 可重放全部。CLI 只用 `faults.rs` 与 `solution.rs`，不读取 starter。
成功退出表示反例/修复重放完成，不能作为学习者 TODO 已完成的证据。

| 输入 | HashMap + `_G` 快照 | 唯一 Heap 环境 |
| --- | --- | --- |
| bare `forward=12`，读 `_G.forward` | `Ok(Nil)` | `Ok(Number(12))` |
| `_G.reverse=34`，读 bare `reverse` | `Ok(Nil)` | `Ok(Number(34))` |
| 两份初始 seed=1，bare seed=nil | `_G.seed` 仍为 1 | 两入口精确 `Ok(Nil)` |
| 两份初始 seed=1，`_G.seed=nil` | bare seed 仍为 1 | 两入口精确 `Ok(Nil)` |

固定删除反例在默认 reference 中分别检查两个方向。不是“两个结果都 truthy”就足够，
要比较准确的数值和 `Ok(Value::Nil)`；Heap 访问失败的 Err 不能冒充 nil 删除成功。

## 唯一 TODO：建立唯一环境 owner

只在 `starter.rs` 实现 `install` 并添加所需导入：分配一张 Heap table，直接初始化
seed 和 `_G` 自指，再返回只包装该 handle 的 Environment：

```diff
- Environment::snapshot(heap)
+ let table = heap.allocate_table()?;
+ heap.table_set(table, key("seed"), Value::Number(1.0))?;
+ heap.table_set(table, key("_G"), Value::Table(table))?;
+ Ok(Environment::from_shared_table(table))
```

这里使用 `model::key` 和 `lua_core::Value`。`from_shared_table` 不创建字段、不扫描
对象，只包装 handle；不要把真正的初始化藏进另一个 helper。快照起点、访问路由和
根枚举在 `model.rs`，先读它们的 owner 与输入/输出，再实现这段初始化。不要在每次
访问时额外同步两份副本，也不要把 `_G` 变成一个特殊的不可写变量。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_exercises single_environment_is_shared_by_globals_and_g -- --ignored --exact
```

初始执行 **1 项且失败**：bare 写入后 `_G.forward` 仍为 nil。完成后同一项通过，合同
还会继续检查反向写入、两个方向的精确 nil 删除、初始自指、重绑 `_G` 与跨 GC 生命周期。
`solution.rs` 是可选参考解；合同、底层实现和默认 ignored 标记不应修改。

```powershell
# 七项默认参考、固定错误有效性与真实 Heap/VM 对照。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_reference
```

默认 workspace 列出 1 项 ignored，不表示学习者已完成；CLI/reference 也不会读取你的
starter，因此练习完成要用上面的显式 `--ignored --exact` 验收。

## `_G` 可以重绑，私有 environment 身份不随之变化

`_G._G == _G` 是**初始安装关系**，不是任意赋值后都成立的语言不变量。
例如执行 `_G = {}` 后：

- bare `name=...` 仍写入 VM 原来的 environment；
- `_G.name=...` 访问新表；这时它与 bare name 已经可以不同；
- 原 environment 的 `'_G'` 字段现在指向新表，私有 `Vm.global_env` 仍是原表。

再把 `_G` 赋 nil，bare globals 仍可读写。模型通过稳定的 owner handle 和真实 Heap
访问检查；真实 VM 用 `original=_G` 保存别名观察旧表，随后把 `_G` 与 `original` 都
赋 nil、结束执行帧、host GC，再验证裸全局写入与 `math.abs` 调用仍可用。不是靠长期
保留这个观察别名来证明环境存活。

## 可达性证据怎样避免隐藏 roots？

模型合同把自环 payload 放入环境，在 `_G=nil` 后收集，实际读取 payload 的 42；
另一个只有 Rust handle 的自环垃圾必须 `is_live==Ok(false)` 且实际访问失败。
删除环境里的 payload 字段，再收集，payload 也必须失效，环境本身继续可访问。

独立真实 Heap 对照先从 env root 保活 `_G` 自环与 child，然后去掉**全部外部 roots**。
即使 Rust 仍保存 env 的 handle 副本，也要求 env/child 的 `is_live==Ok(false)` 和实际
table 访问失败。自环是边，不能自行成为 root。

真实 VM 另有三项探针：

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_reference real_vm_global_and_g_writes_and_nil_deletions_share_storage -- --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_reference real_vm_environment_keeps_reachable_graph_across_execute_and_gc -- --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_reference real_vm_rebinding_g_does_not_replace_the_environment -- --exact
```

第二项分多次 execute，只返回布尔值，先通过环境字段保活 self-cycle payload/child，
host GC 后实际读取 child 的 42；下一次仅删除 bare `shared` 并结束帧，再 host GC，
要求全局弱表的 owner/child 两槽精确 nil。这样不会把上一次观察表达式留下的活动
寄存器误当成额外 root。三项探针都不读取 VM 私有字段，环境身份由可观察路由行为验证。

本题使用无元表的普通环境字段和现成 GC API，不覆盖所有元方法、coroutine、活动
varargs、尾调用参数、GC phase 或错误恢复路径，不能据此声称全部 GC 路径安全。
永久复现入口为此 README 与 `model.rs`、`contracts.rs`、`real.rs`、`reference.rs`。
