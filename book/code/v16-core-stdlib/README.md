# v0.16 · 核心标准库

本快照新增 `lua_stdlib` crate，集中管理 Rust builtin 目录、Lua coercion 和一基参数错误。全局环境成为真正的 Heap table，`_G`、普通 global 和 math/string/table/coroutine 库共享同一个对象。

```powershell
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml -- `
  book/code/v16-core-stdlib/examples/demo.lua
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --workspace
```

本版公开 19 个 base、28 个 math、10 个 byte-string、5 个 table 与 6 个 coroutine 入口。`pcall/xpcall`、string method、table comparator、`collectgarbage` 以及 `_VERSION`/`_G` 都有端到端合同测试，并在存在 `lua` 命令时运行代表性 Lua 5.1 差分。

## 三条可运行的问题链

从仓库根目录运行；每个 CLI 固定并列错误与参考，不读取学习者 starter：

```powershell
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml -- book/code/v16-core-stdlib/examples/builtin_contracts.lua
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example args_problem_chain
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example catalog_problem_chain
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example globals_problem_chain
```

Lua 探针应返回 `type=boolean value=true`。各 CLI 的模式、固定输出、精确验收命令与修改范围见：

- [参数规则](exercises/args/README.md)：两个独立 TODO，数字字符串/一基错误与拥有的 bytes；真实 Args/VM 对照。
- [集中注册](exercises/catalog/README.md)：一个 installer TODO，漏注册/错 ID/重名；独立 68 项名字/ID 清单及代表调用效果。
- [唯一全局环境](exercises/globals/README.md)：一个 initializer TODO，双向写/删除/GC 与 `_G` 重绑；真实 Heap/VM 对照。

这三包是人为局部故障设计，不是历史中间 VM。目录实验没有把自己的 Heap 安装进真实
VM：它读出 NativeFunction ID 后通过固定字节码探针调用，真实 VM 初始化另有对照。
重复名 preflight 是教学保护；`gc_*` 诊断仍另行安装。`_G` 是可变字段，VM 保留原环境身份。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_exercises -- --ignored
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_exercises -- --ignored
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test globals_exercises -- --ignored
```

三条 learner 命令初始分别 2/1/1 项失败；只修改各包 starter 的对应 hook 后通过。默认
workspace 的 ignored 不代表练习已完成，固定 CLI 退出 0 也只表示反例重放成功。
维护者验证需要临时套解并按字节恢复，再重跑初始失败。

```powershell
cargo fmt --manifest-path book/code/v16-core-stdlib/Cargo.toml --all -- --check
cargo clippy --manifest-path book/code/v16-core-stdlib/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --workspace --release
```

正向合同只说明上述范围。v0.15 交接中已记录活动 varargs/tail-call 漏根，本教学包没有
修复该工程问题，也不宣称全部 GC 路径正确。

## 保护调用、禁止 yield 与排序临时 roots

后续三包把结果交付、执行状态与可达性分开验收：

- [保护结果](exercises/protected/README.md)：两个 TODO，保留 pcall 多值/nil，xpcall 仅错误时调用 handler 并取首结果。
- [边界恢复](exercises/boundary/README.md)：一个 TODO，普通 Ok/Err 后恢复进入前 depth，内层不能清除外层禁止状态。
- [排序 roots](exercises/sort_roots/README.md)：一个 TODO，以真实 Heap 验证全快照保活、错误后截断及外层 prefix 保留。

```powershell
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml -- book/code/v16-core-stdlib/examples/callback_contracts.lua
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example protected_problem_chain
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example boundary_problem_chain
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example sort_roots_problem_chain
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test boundary_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test sort_roots_reference
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test protected_exercises -- --ignored
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test boundary_exercises -- --ignored
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test sort_roots_exercises -- --ignored
```

Lua 探针最终返回 true：被保护调用拒绝的 yield 之后，同一 coroutine 仍可合法 yield
41,nil，再恢复返回 43。精确 roots/depth 前缀由局部合同验收；同一 execute 的正向行为
不作为整个 VM 私有状态清理的证明。三包 learner 初始分别 2/1/1 项失败，均为普通
Result 协议练习，不承诺 panic/abort 安全。固定 CLI 不读取 starter。

本轮增加 protected 9、boundary 8、sort_roots 5 项参考，与前一批 133 项合计
**155 通过、8 ignored**。八项 learner 均为显式待完成练习，独立已知失败输入不在计数中。

**当前真实功能缺口，未修复：**

```powershell
# 预期当前失败，退出 2，错误包含 stale GC handle。
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml -- book/code/v16-core-stdlib/examples/sort_snapshot_roots_gap.lua
```

该输入结束构造帧后对四个 rank table 排序，comparator 清空源表并收集，未参与当前
比较的对象只剩 Rust snapshot handle，后续访问 stale。此项单独记为 known-functional-failure，
不计入默认参考成功或 ignored；sort_roots 的健康方案是独立 Heap 实验，真实 VM 的
snapshot 尚未登记全根。原 v0.15 活动 varargs 缺口亦未修复。

Lua pattern、完整 `string.format`、chunk loader、function environment 和部分 collector tuning 仍留待后续兼容收口；io/os/package/debug 属于 v0.17。
