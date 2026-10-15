# 已经打印错误，为什么 shell 仍把程序当成功？

先预测：`main() -> ()` 中只调用 `eprintln!`，进程会退出 1 吗？如果把所有错误都
改成 1，unknown option 与执行期间的错误还能区别吗？如果成功时自动打印 Lua
返回值，`print('kept'); return 42` 的 stdout 又会多出什么？

本包只设一个 `starter.rs::finish` TODO；默认 8 项参考、1 项 ignored learner。
它是进程边界的小模型，不是历史中间版本，也没有修改 v0.18 的真实 CLI。

## 先构建真实 binary，再运行固定对照

从仓库根运行；真实进程模式必须先构建 binary，example 不会自行启动 nested cargo：

```powershell
$cliManifest = 'book/code/v18-cli-tools/Cargo.toml'
cargo build --manifest-path $cliManifest --bins
cargo run --quiet --manifest-path $cliManifest --example cli_exit_problem_chain
cargo test --manifest-path $cliManifest --test cli_exit_reference
cargo test --release --manifest-path $cliManifest --test cli_exit_reference
```

CLI 缺省为 `all`，也可选择 `error`、`streams`、`real`；固定使用 faults/solution，
不读取 starter。`all` 共 25 行：8 行模型对照和 17 个真实 child case。example 根据
`current_exe` 的 profile sibling 定位 `lua_app(.exe)`；缺失时明确报路径与 build
命令，不会静默换用另一处已安装 Lua。integration 另用 Cargo 提供的
`env!("CARGO_BIN_EXE_lua_app")`，Debug/Release 各自运行对应 profile 的真实程序。

## 一次决定三个可观察结果

`Outcome` 拥有已经产生的 stdout bytes 和 `Result<Vec<Value>, CliError>`。
`finish(Outcome) -> Transcript` 消费它，交付退出码、stdout、stderr：

| 结果 | exit | stdout | stderr |
| --- | --- | --- | --- |
| Ok | 0 | 保留已产生的 bytes，不追加 script return | 空 |
| Usage | 2 | 保留已产生的 bytes | `lua: message`，随后 usage |
| Runtime | 1 | 保留已产生的 bytes | `lua: message`，没有 usage |

模型中的 Runtime 对应真实 CLI 的分类：`AppError::Compile` 也经过 `runtime_error`
变为 `CliError::Runtime`；读文件失败直接进入同一类。因此“语法错误”不是参数用法
错误，不应因为它在执行前出现就给退出码 2。`--version` 是成功 action；`--help`
在真实 parse_options 内直接打印 usage 并 `process::exit(0)`，不是 Usage Err。

固定错误 `print_error_but_succeed` 已正确生成 stderr，却仍给 0；`all_errors_one`
又把 Usage 压成 1。先运行 `error`，观察同一个 message 的 code 与流，不只读文字：

```text
before print-only usage: exit=0 ...
before all-one usage: exit=1 ...
after classified usage: exit=2 ...
after classified runtime: exit=1 stdout="kept\n" stderr="lua: broken\n"
```

这段 Result 模型并不实现 Lua print 拦截器。`Outcome.stdout` 是测试预先提供的 bytes，
用来表达“边界不回滚已经输出的内容”；真实 Lua 的 print 是否正确进入 stdout，要看
独立 child process 捕获。模型也不证明输出系统调用失败、终端编码或 panic 的合同。

## 唯一 TODO：匹配错误类别并保留已有 stdout

只修改 `starter.rs::finish`；返回 Transcript，不直接写测试进程的 stdout/stderr。

```diff
-error_transcript(outcome, 0, 0)
+match outcome.result {
+    Ok(_) => /* exit 0, stderr empty */,
+    Err(CliError::Usage(message)) => /* exit 2, message + usage */,
+    Err(CliError::Runtime(message)) => /* exit 1, message only */,
+}
+// stdout 始终使用 outcome.stdout，不改写既有 bytes。
```

```powershell
cargo test --manifest-path $cliManifest --test cli_exit_exercises result_boundary_preserves_streams_and_maps_exit_codes -- --ignored --exact
cargo test --manifest-path $cliManifest --test cli_exit_reference result_boundary_preserves_streams_and_maps_exit_codes -- --exact
```

初始应恰好执行 1 项并失败；修复后同一合同通过。共享合同交叉覆盖空 stdout、已有
换行、NUL/0xff bytes；成功返回空窗口、nil、多值；Usage 与 Runtime 的退出码及精确
stderr。不要修改 ignored、合同、真实源码或固定 fault 来取得通过。

## 为什么脚本 return 不应该变成 stdout

`faults::print_script_results` 将 `return 42` 自动追加为 `42\n`，结果是
`"kept\n42\n"`；健康脚本边界只保留 `"kept\n"`。真实 `run` 中 -e/script 调用
`execute_multi` 后丢弃返回窗口，Lua 显式 print 的输出保留；交互 REPL 则有独立的
`print_values` 协议，本题没有把它改成静默。不要把 script 与 REPL 的输出合同合并。

## 真实 child case：错误后还运行了什么

`real.rs` 不调用模型 hook。四组真实参考覆盖 17 次独立启动：

- --help/-h、--version/-v 成功且 stderr 空；unknown option、缺 -e 或 -l operand
  为 exit 2、stdout 空、stderr 含 usage 与具体原因。
- 编译、运行与 missing file 为 exit 1，stderr 有 `lua:` 且没有 usage；`print` 后
  error 仍保留已输出的 `kept\n`。stdin、-e、script 的成功 return 42 都不会自动打印。
- -e 或 -l action 失败后，后续 -e、script、-i 都不执行。所有选项都放在 script
  参数之前；提供的 stdin 真包含 REPL print，stdout 精确等于 `kept\n`，因此可发现
  误进入 REPL 时的 banner/prompt 或额外文本。
- script 执行失败后不进入 -i；missing file 之前已执行的 -v 输出仍在，也不出现
  第二份交互 banner。这区分“停止后续工作”与“回滚之前的输出”。

```powershell
cargo test --manifest-path $cliManifest --test cli_exit_reference real_process_help_version_unknown_and_missing_operand_streams -- --exact
cargo test --manifest-path $cliManifest --test cli_exit_reference real_process_compile_runtime_file_failures_and_silent_script_returns -- --exact
cargo test --manifest-path $cliManifest --test cli_exit_reference real_process_failed_execute_or_load_stops_later_actions_script_and_repl -- --exact
cargo test --manifest-path $cliManifest --test cli_exit_reference real_process_failed_script_or_missing_file_stops_repl -- --exact
```

## 共享进程 helper 的边界

三包共享 `../support/process.rs`：两个 reader 线程分别 drain stdout/stderr；writer
线程写完 stdin 后 drop pipe，主动交付 EOF；主线程按五秒 deadline 用 try_wait 轮询。
timeout 会 kill/wait 所拥有的那个 child，再 join 流线程，并返回 helper Err，不把
超时伪装成 Lua 自己返回的某个退出码。早退造成的 stdin BrokenPipe 可接受，但退出码
与输出仍由每个 probe 单独核对。

fixture 输出有界、不会启动后代进程；helper 只管理一个 exact child，不承诺整个
process tree 终止协议或后代继承 pipe 时的 join 上界。没有把旧 cli_contract.rs 中
“先 wait、再读 pipe”的 small-fixture runner 当成本包并发 drain 的证据，也没有用
超大输出来制造测试。

临时目录唯一创建；清理前 canonicalize、检查父目录、原始唯一目录名和前缀，正常
finish 合并原错误与清理错误，Drop fallback 只报告而不 panic。所有 child 使用独立
命令参数与 pipe，不修改测试进程的 cwd 或全局环境。

理解题：为什么 stderr 非空不能推出 exit 非零？
实现题：完成 finish，显式 learner 恰好执行 1 项并通过。
调试题：自动打印 script return 以及 action 失败后继续 -i，各会在哪条精确 stdout
断言中暴露？再解释为什么已有 print 没有因后面的 error 消失。
