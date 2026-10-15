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
修复该工程问题，也不宣称全部 GC 路径正确；受保护 callback/排序资源保活另作下一包。

Lua pattern、完整 `string.format`、chunk loader、function environment 和部分 collector tuning 仍留待后续兼容收口；io/os/package/debug 属于 v0.17。
