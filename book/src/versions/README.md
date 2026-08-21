# 版本主线：从 0 到 Lua 5.1

<div class="lesson-meta"><span>20 个可运行 checkpoint</span><span>Rust 初中级</span><span>循序渐进</span></div>

这条主线不会在第一章就把完整解释器藏在框架后面。每个版本只增加一个核心能力，且对应一份能独立编译、运行和测试的代码快照。你可以从任意 checkpoint 开始，不需要依赖 Git 标签或历史提交。

## 怎样使用快照

每份快照都位于 `book/code/vXX-name/`，且自带 `Cargo.toml`、`Cargo.lock`、源码、测试和示例：

```powershell
cargo run --manifest-path book/code/v04-expressions/Cargo.toml -- `
  book/code/v04-expressions/examples/demo.lua
cargo test --manifest-path book/code/v04-expressions/Cargo.toml
```

一次验证当前已落地的所有快照：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File book/code/check.ps1
```

## 实施状态

| 版本 | 唯一核心增量 | 形态 | 状态 |
|---|---|---|---|
| [v0.0](v00-skeleton.md) | CLI、输入与错误出口 | 单 crate | 已实现 |
| [v0.1](v01-value.md) | `Value` 与字节字符串 | 单 crate | 已实现 |
| [v0.2](v02-lexer.md) | Lexer 与 Token | 单 crate | 已实现 |
| [v0.3](v03-parser.md) | Parser 与最小 AST | 单 crate | 已实现 |
| [v0.4](v04-expressions.md) | 完整表达式语义 | 单 crate | 已实现 |
| [v0.5](v05-bytecode-vm.md) | Bytecode 与最小寄存器 VM | 4-crate workspace | 已实现 |
| [v0.6](v06-variables-scope.md) | 变量、赋值与作用域 | workspace | 已实现 |
| [v0.7](v07-control-flow.md) | 控制流与跳转回填 | workspace | 已实现 |
| [v0.8](v08-table.md) | Table 与共享可变性 | workspace | 已实现 |
| [v0.9](v09-functions.md) | 函数与调用帧 | workspace | 已实现 |
| [v0.10](v10-multi-return-vararg.md) | 多返回、vararg 与迭代器 | workspace | 已实现 |
| [v0.11](v11-closure-upvalue.md) | Closure 与 Upvalue | workspace | 已实现 |
| [v0.12](v12-metatable.md) | Metatable 与元方法 | workspace | 已实现 |
| [v0.13](v13-runtime-gc.md) | Runtime 所有权与完整 GC | workspace | 已实现 |
| [v0.14](v14-incremental-gc.md) | 增量 GC、弱表与 Finalizer | workspace | 已实现 |
| [v0.15](v15-coroutine.md) | Coroutine | workspace | 已实现 |
| [v0.16](v16-core-stdlib.md) | 核心标准库 | + `lua_stdlib` | 已实现 |
| [v0.17](v17-host-stdlib-modules.md) | 宿主标准库与 Lua 模块 | workspace | 已实现 |
| [v0.18](v18-cli-tools.md) | CLI、REPL 与字节码工具 | + `lua_bytecode` | 已实现 |
| [v1.0](v100-lua51-runtime.md) | Lua 5.1 语言运行时兼容收口 | release + 双 oracle | 已实现 |

`已实现` 表示对应教学快照、本章验收与 CI 检查列表均已落地。v1.0 进一步使用闭世界能力矩阵区分 `complete`、`partial` 和 `excluded`；代表性双 oracle 基线通过不等于官方全量测试套件通过，所有剩余差异都必须引用偏差 ID。

## 架构为什么会中途改变

v0.0–v0.4 刻意使用单 crate：读者可以把注意力放在语义上。v0.5 开始出现 Proto、指令编码和 VM 调度，届时再拆成 `lua_core`、`lua_compiler`、`lua_vm` 和 `lua_app`。这次迁移本身就是教学内容，而不是提前给出一个空洞的“最佳架构”。

## v1.0 的边界

v1.0 覆盖 Lua 5.1 源码语言、VM、GC、协程、标准库教学子集、Lua 文件模块和 CLI，并冻结兼容证据与偏差治理。Lua 5.1 binary chunk 读写、稳定 C API/ABI 和动态原生模块分别属于 v1.1、v1.2 和 v1.3，不会在 v1.0 中被静默地宣称为已支持。

完整长期规范见仓库根目录的 [`book_plan.md`](https://github.com/YanqingXu/lua_rust/blob/main/book_plan.md)。现有 Value、Lexer、VM 等章节保留为“专题深入”，可以在完成对应 checkpoint 后阅读。
