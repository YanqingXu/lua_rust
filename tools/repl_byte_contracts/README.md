# 独立快照的 REPL 字节合同

`cases.tsv` 是手写的固定 oracle，不读取任何运行输出来生成预期。前 21 条沿用
v0.18 结构化 REPL 的验收矩阵，另 5 条明确约束 CRLF 和末片段无 LF。v0.18、v1.0
各带一份相同 fixture、相同测试与相同 runner；复制某一个快照后能独立构建，不导入
相邻版本的 Rust 文件或 target 产物。

从仓库根运行同一性与格式检查：

```powershell
py -3 tools/check_repl_byte_contracts.py --check
```

每行七列，以 `|` 分隔：case id、逗号分隔的 argv hex、stdin hex、exit、
banner 策略、stdout body hex、stderr hex。hex 是小写的原 bytes，空字段表示空数据。
banner 只允许 `none` 与 `repl`；`repl` 的预期只前缀固定版本 banner。实际输出
不 trim、不作 UTF-8 有损解码、不改 CRLF，也不忽略诊断。两份实现都须各自符合
oracle，不能只做相互比较。

两个快照分别执行本地集成测试（Release 加 `--release`）：

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_byte_contract real_binary_matches_all_shared_raw_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_byte_contract real_binary_matches_all_shared_raw_byte_cases -- --exact
```

构建两边 binary 后，还可用独立 Python runner 再核对同一固定 corpus，传入两边实际
可执行文件路径；Cargo 使用自定义 target-dir/target triple 时据构建产物调整路径：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --bin lua_app
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app
py -3 tools/check_repl_byte_contracts.py --v18-bin book/code/v18-cli-tools/target/x86_64-pc-windows-msvc/debug/lua_app.exe --v100-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --output target/repl-byte-contracts
```

每次运行创建独占目录，保存每个 case 的 stdin、实际和预期双流 bytes、状态与汇总；
summary 包含 corpus/本地 harness 与实际 binary 的 SHA-256。缺文件或写失败会失败，
不覆盖旧证据，也不声称归档具备 fsync/断电原子性。Rust runner 同时 drain 两流，写完
关闭 stdin，5 秒 deadline 后 kill/wait 并 join；Python 使用 communicate 并在超时后
kill 再 communicate/wait。本合同输入输出有界、无后代，不证明任意进程树清理。

case 覆盖 idle/pending EOF、普通语法错误恢复、成功恰一次和前缀零执行、长短字符串、
表达式续行、无 LF、CRLF、NUL/0xff、one-shot 不续行。可注入的宿主 I/O 读写错误由
各快照的 `repl_input_protocol` 测试另验；不依赖操作系统的错误措辞，也不声称与官方
Lua 的完整 TTY REPL 等价。修改语言协议时先审查 oracle，再同步两份副本并重跑全部 gate。
