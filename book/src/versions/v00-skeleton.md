# v0.0：最小解释器骨架

<div class="lesson-meta"><span>Checkpoint 1 / 20</span><span>CLI 与 I/O</span><span>单 crate</span></div>

## 从什么都不会到可验证的边界

此前没有解释器。本版本的唯一核心增量是建立 `输入 → 执行器 → 输出/错误` 链路：命令行可以传入一个脚本路径，不传路径时从 stdin 读取，所有失败经过同一个 stderr 出口并返回非零退出码。

这一版不解析 Lua。`return 42` 与一段随机字节对执行器来说没有区别；它只能报告输入来源与字节数。这个限制是公开接口的一部分，不会偷偷调用成熟解释器。

## Rust 学习点

- `Cargo.toml` 中的 library/binary 分层；
- `OsString` 保留操作系统路径，`Vec<u8>` 保留脚本原始字节；
- `Result<T, E>` 将 I/O 失败传到唯一错误出口；
- `ExitCode` 把成功与失败变成可被 shell 和 CI 观察的合同。

## 实现设计

`load_source` 只负责输入，`run` 只接收已加载的 `SourceChunk`。这条小小的边界会在后续保留：Lexer 不应该知道字节来自文件还是 stdin。

```rust
{{#include ../../code/v00-skeleton/src/lib.rs:input_boundary}}
```

另一种做法是让 `main` 中的每个分支直接读取并执行。代码会短几行，但无法在不启动子进程的情况下测试 stdin 和参数规则。因此这里同时保留纯逻辑测试与进程级 CLI 测试。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v00-skeleton/Cargo.toml -- `
  book/code/v00-skeleton/examples/demo.lua
```

预期输出：

```text
chunk=book/code/v00-skeleton/examples/demo.lua bytes=75
```

不存在的文件必须将错误写入 stderr，且退出码为 `2`。完整验收：

```powershell
cargo test --manifest-path book/code/v00-skeleton/Cargo.toml
```

测试覆盖原始 NUL 字节、多余参数、stdin 与文件不存在。

## 本版本暂不支持

- 任何 Lua 词法或语法；
- Value、环境、语句和输出函数；
- REPL、`-e` 等完整 CLI 选项。

## 练习与迁移

1. 给 `RunSummary` 增加行数，注意 `\r\n` 如何计数。
2. 增加“只允许一个路径”的进程级测试。
3. 解释为什么脚本源码用 `Vec<u8>` 而不是 `String`。

迁往 v0.1 时保留 CLI 和 `SourceChunk`，新增 `Value`、`ByteString` 与单字面量求值器；`run` 不再只返回字节数。成熟实现的对应入口是 `crates/lua_app`，但它包含后续能力，此时只做对照阅读。
