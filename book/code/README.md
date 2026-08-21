# 教学代码快照

这里保存《从 Lua 解释器开始学习 Rust》的独立代码 checkpoint。每个目录都拥有自己的
Cargo 配置、示例与测试，不依赖 Git 历史即可运行。

当前已落地的基础闭环：

| 快照 | 能力 | 运行入口 |
|---|---|---|
| `v00-skeleton` | CLI、文件/stdin、统一错误出口 | `cargo run --manifest-path book/code/v00-skeleton/Cargo.toml -- book/code/v00-skeleton/examples/demo.lua` |
| `v01-value` | Lua Value、ByteString、单字面量 | `cargo run --manifest-path book/code/v01-value/Cargo.toml -- book/code/v01-value/examples/demo.lua` |
| `v02-lexer` | byte-oriented Lexer 与 Token | `cargo run --manifest-path book/code/v02-lexer/Cargo.toml -- book/code/v02-lexer/examples/demo.lua` |
| `v03-parser` | 表达式 AST、优先级与最小 evaluator | `cargo run --manifest-path book/code/v03-parser/Cargo.toml -- book/code/v03-parser/examples/demo.lua` |
| `v04-expressions` | Lua 表达式、短路、比较、拼接与长度 | `cargo run --manifest-path book/code/v04-expressions/Cargo.toml -- book/code/v04-expressions/examples/demo.lua` |
| `v05-bytecode-vm` | 32-bit Bytecode、Proto、Codegen 与寄存器 VM | `cargo run --manifest-path book/code/v05-bytecode-vm/Cargo.toml -- --dump-bytecode book/code/v05-bytecode-vm/examples/demo.lua` |
| `v06-variables-scope` | global/local、并行赋值与词法作用域 | `cargo run --manifest-path book/code/v06-variables-scope/Cargo.toml -- book/code/v06-variables-scope/examples/demo.lua` |
| `v07-control-flow` | 分支、循环、numeric for 与 break 回填 | `cargo run --manifest-path book/code/v07-control-flow/Cargo.toml -- book/code/v07-control-flow/examples/demo.lua` |
| `v08-table` | 数组/哈希混合 Table 与共享对象 | `cargo run --manifest-path book/code/v08-table/Cargo.toml -- book/code/v08-table/examples/demo.lua` |
| `v09-functions` | Lua Function、嵌套 Proto、调用帧与递归 | `cargo run --manifest-path book/code/v09-functions/Cargo.toml -- book/code/v09-functions/examples/demo.lua` |
| `v10-multi-return-vararg` | 多返回、vararg、尾调用与迭代器 | `cargo run --manifest-path book/code/v10-multi-return-vararg/Cargo.toml -- book/code/v10-multi-return-vararg/examples/demo.lua` |
| `v11-closure-upvalue` | Closure、共享 Upvalue 与作用域关闭 | `cargo run --manifest-path book/code/v11-closure-upvalue/Cargo.toml -- book/code/v11-closure-upvalue/examples/demo.lua` |
| `v12-metatable` | Metatable、元方法与动态语义分发 | `cargo run --manifest-path book/code/v12-metatable/Cargo.toml -- book/code/v12-metatable/examples/demo.lua` |
| `v13-runtime-gc` | Runtime 唯一所有权、GcRef 与完整 mark/sweep | `cargo run --manifest-path book/code/v13-runtime-gc/Cargo.toml -- book/code/v13-runtime-gc/examples/demo.lua` |
| `v14-incremental-gc` | 增量 phase、barrier、弱表与 finalizer | `cargo run --manifest-path book/code/v14-incremental-gc/Cargo.toml -- book/code/v14-incremental-gc/examples/demo.lua` |

统一验证入口：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File book/code/check.ps1
```
