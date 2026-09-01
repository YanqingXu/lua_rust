# v0.0：最小解释器骨架

<div class="lesson-meta"><span>Checkpoint 1 / 20</span><span>CLI 与 I/O</span><span>单 crate</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>这一版只建立程序边界</strong>
  <p>前置：能运行 <code>cargo new</code>。新知识：library/binary 分工、值所有权、
  <code>Result</code>、<code>?</code>、stderr 与退出码。完成后你应能从参数或 stdin 读取
  原始字节，并让失败沿一条可测试的路径返回。</p>
</div>

本版不要求理解 Lua。请先把注意力放在三个问题上：源码字节由谁拥有、I/O 错误经过
哪些函数、shell 如何区分成功和失败。

<a id="learning-predict"></a>
## 先预测再运行

先不要看实现。分别预测下面两条命令的 stdout、stderr 和退出码：

```powershell
cargo run --manifest-path book/code/v00-skeleton/Cargo.toml -- `
  book/code/v00-skeleton/examples/demo.lua
cargo run --manifest-path book/code/v00-skeleton/Cargo.toml -- missing.lua
```

第一条命令不应解释 `demo.lua`，只报告来源和字节数；第二条命令必须把带路径的错误写到
stderr。这个可观察合同就是随后所有设计的起点。

## 从什么都不会到可验证的边界

此前没有解释器。本版本的唯一核心增量是建立 `输入 → 执行器 → 输出/错误` 链路：命令行可以传入一个脚本路径，不传路径时从 stdin 读取，所有失败经过同一个 stderr 出口并返回非零退出码。

这一版不解析 Lua。`return 42` 与一段随机字节对执行器来说没有区别；它只能报告输入来源与字节数。这个限制是公开接口的一部分，不会偷偷调用成熟解释器。

## Rust 学习点

- `Cargo.toml` 中的 library/binary 分层；
- `OsString` 保留操作系统路径，`Vec<u8>` 保留脚本原始字节；
- `Result<T, E>` 将 I/O 失败传到唯一错误出口；
- `ExitCode` 把成功与失败变成可被 shell 和 CI 观察的合同。

<a id="learning-code-map"></a>
## 代码阅读地图

按调用顺序阅读，不要从文件第一行漫无目的地向下翻：

1. `src/main.rs::main`：把成功或失败转换成 `ExitCode`；
2. `src/lib.rs::load_source`：决定文件与 stdin 两条输入路径；
3. `SourceChunk`：拥有 chunk name 和 `Vec<u8>`；
4. `run(&SourceChunk)`：只借用输入，返回可打印摘要；
5. `tests/cli.rs`：从进程外观察 stdout、stderr 和退出码。

注意所有权方向：`load_source` 创建并返回 `SourceChunk`，调用者成为 owner；`run` 只借用
它，所以执行结束后仍可读取名称和字节。若 `run` 接收 `SourceChunk` 本身，调用者就不能
再检查输入，这里没有必要转移所有权。

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

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>解释为什么脚本源码使用 <code>Vec&lt;u8&gt;</code>，路径参数使用
  <code>OsString</code>，而不是把两者都转换成 <code>String</code>。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>给 <code>RunSummary</code> 增加行数，并为 LF、CRLF、空输入各写一个测试。修改范围只限
  <code>src/lib.rs</code>；运行本快照的 <code>cargo test</code> 作为成功条件。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>暂时删除“多余参数”检查，运行 <code>extra_arguments_are_rejected</code>，沿测试失败信息
  找到参数合同被破坏的位置，然后恢复检查。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出 `main → load_source → run → stdout/stderr` 调用链；
- 能说明 `SourceChunk` 的 owner 以及 `run` 为什么只需共享借用；
- 能写一个同时断言 stderr 和非零退出码的进程测试。

## 从本版迁往 v0.1

迁往 v0.1 时保留 CLI 和 `SourceChunk`，新增 `Value`、`ByteString` 与单字面量求值器；`run` 不再只返回字节数。成熟实现的对应入口是 `crates/lua_app`，但它包含后续能力，此时只做对照阅读。
