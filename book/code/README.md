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

## 在 VS Code 中调试 Rust

用 VS Code 打开**仓库根目录** `lua_rust`，安装 `rust-lang.rust-analyzer` 和
`vadimcn.vscode-lldb`（CodeLLDB）。根目录的 [.vscode/launch.json](../../.vscode/launch.json)
已提供教学快照入口，按 `Ctrl+Shift+D` 选择，再按 `F5`：

| 调试入口 | 选择与默认行为 |
|---|---|
| `Debug book/code (v00-v04 demo.lua)` | 选择 v00–v04，构建 `lua-step`，运行所选版本的 `examples/demo.lua` |
| `Debug book/code (v05-v100 demo.lua)` | 选择 v05–v100，构建 `lua_app`，运行所选版本的 `examples/demo.lua` |
| `Debug book/code (Rust example)` | 选择版本、`examples/*.rs` 的文件名（不带 `.rs`），默认程序参数为 `all` |
| `Debug book/code (Rust unit test)` | 选择版本、package 和单元测试完整名称，精确运行一项 `--lib` 测试 |

例如调试 v1.0 的解释器：

1. 在 `book/code/v100-lua51-runtime/src/main.rs` 的 `main` 内设断点。
2. 选择 `Debug book/code (v05-v100 demo.lua)`，按 `F5`，选择 `v100-lua51-runtime`。
3. 停住后用 `F10` 单步跳过、`F11` 进入函数，在 Variables / Watch 中查看 Rust 值。
   需要跟进编译器或 VM 时，在**该快照** `crates/` 下的 Rust 源码里设断点。

这里的 Lua 文件是解释器输入；断点设在 `.rs` 中。每个快照的源码和根目录 `crates/`
是不同副本，必须与启动时选择的版本一致。

`Rust example` 的默认组合是 `v100-lua51-runtime`、`string_format_problem_chain`，
参数为 `all`，可在该版本的 `examples/string_format_problem_chain.rs` 里设断点。
其它版本要填写实际存在的 Rust 示例；需要其它参数时修改该配置顶层 `args` 数组，
每项为一个参数，不需要参数时改为 `[]`。

`Rust unit test` 的默认组合是 v1.0 的 `lua_vm` package +
`string_format::tests::spec_cursor_and_empty_precision_are_explicit`，断点可设在
`crates/lua_vm/src/string_format.rs` 的同名测试内。换测试时填写包含模块路径的全名；
如果显示 `running 0 tests`，请核对版本、package 和完整名称。

配置使用 CodeLLDB 的 Cargo 支持：`--manifest-path` 指定快照，`--bin` / `--example` /
`--lib` 指定构建入口，由 Cargo 返回实际可执行文件路径；不需要手填 `.exe` 或测试
哈希文件名，也不需要另加 `preLaunchTask`。这是独立 workspace，根目录的
`cargo build -p lua_app` 只构建根工程，不能替代快照构建。
参见 [CodeLLDB Cargo 支持](https://github.com/vadimcn/codelldb/blob/v1.12.3/MANUAL.md#cargo-support)。

构建和被调试程序的 `cwd` 均为仓库根目录，与本页运行命令一致；例如 v0.17 的
demo 中 `package.path` 使用仓库相对路径。Windows 继承根目录 `.cargo/config.toml` 的
MSVC target 和栈配置，产物通常位于各快照自己的
`target/x86_64-pc-windows-msvc/debug/`。默认使用含调试信息的开发/测试构建。

调试自己的 Lua 脚本时，可复制对应 demo 配置，把顶层 `args` 改为脚本绝对路径。
也可用 `["${file}"]`，但启动时必须让该 `.lua` 文件处于活动编辑器；如果当前打开
的是 Rust 文件，就会把 Rust 源码误当作 Lua 输入。调试 v0.18 / v1.0 REPL 时，将
顶层 `args` 改为 `[]`，在集成终端输入；较早版本无参数时通常读取 stdin 直到 EOF。

如需调试集成测试，复制单元测试配置，将 Cargo 的 `--lib` 改为
`--test <目标名>`，并将 `cargo.filter` 改为 `{ "name": "<目标名>", "kind": "test" }`。
测试运行时的筛选参数仍放顶层 `args`。若集成测试会启动解释器子进程，父测试的
调试器不会自动跟进子进程；解释器内部断点应使用 demo 配置直接启动 `lua_app`。

### 让 rust-analyzer 识别当前学习版本

若教学源码提示不属于已加载工程、跳转或测试旁的 `Debug` 入口缺失，可在 VS Code 的
工作区设置 `.vscode/settings.json` 中显式关联当前版本，例如：

```json
{
    "rust-analyzer.linkedProjects": [
        "Cargo.toml",
        "book/code/v100-lua51-runtime/Cargo.toml"
    ]
}
```

将第二项换成正在学习的版本；一个 workspace 的根 manifest 会带上其成员 crate。
`linkedProjects` 会替代自动发现，因此这里也保留根工程。保存后可执行
`rust-analyzer: Reload Workspace`。无需同时加载全部 20 份解释器。
该设置影响编辑器分析，F5 调试入口仍按启动时选择的快照构建。
参见 [rust-analyzer 配置](https://rust-analyzer.github.io/book/configuration.html#rust-analyzer.linkedProjects)。
