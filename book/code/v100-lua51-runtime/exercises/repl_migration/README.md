# 独立快照怎样迁移 REPL 协议？

先预测：v0.18 已能把 `return 40 +` 与下一行 `2` 合为42，为什么 v1.0 仍可能先
报错？两个目录各自拥有 compiler、library 和 binary；修改前一版本不会改变后一个。
本包调用 v1.0 本地真实 API，练习一个 EOF 所有权 TODO，再用同一字节合同核对两端。

从仓库根运行固定反例与参考实现：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example repl_migration_problem_chain -- all
```

模式为 `faults|migration|all`，缺省 all；`all` 有12条不同的 `[ok]`。示例只运行
固定 faults/solution 与真实 Repl，不读取 starter，不启动嵌套 cargo。包内参考6项、
learner1项 ignored；此外 `tests/repl_input_protocol.rs` 有6项实际 controller/I/O
回归。字节级进程合同另由 `tests/repl_byte_contract.rs` 执行，不把本例输出 marker
当成进程 stdout/stderr 的完整验收。

## 问题一：迁移消息判断，还是迁移结构化协议？

迁移前本地 `main::is_incomplete` 搜索 `expected 'end'`、`unexpected end` 和
`unterminated`。真实编译器对 `return 40 +\n` 报 `expected an expression`，关键词
没有命中，但下一行确实能补齐表达式。固定 fault 调用本地 compiler 重现漏判；另对
Invalid 错误只改 message，证明措辞不能改变 kind。后一个是明确的措辞注入实验，
不是声称编译器会真的打印那段诊断。

```diff
-Err(AppError::Compile(error)) if is_incomplete(&error) => continue,
+Err(AppError::Compile(error)) if error.kind == CompileErrorKind::NeedMoreInput => {
+    self.pending = Some(error);
+    Ok(ReplEvent::NeedMoreInput)
+}
```

先迁移本地 compiler 的 `CompileErrorKind`、lexer/parser 构造点与测试，再迁移本地
`src/lib.rs::Repl/run_repl`，最后把本地 main 改为 I/O adapter。Rust 源码和 path 依赖
都留在 v1.0 目录；没有让 v1.0 编译引用 v0.18 的 compiler、controller 或 VM。
v1.0 的 `Action::Help` 仍在正常 action 流返回，版本文字仍为 v1.0，`-e/-l/-v/-i`
顺序、arg 表与已有 runtime API 均保留，不能为复用 REPL 把整个 main 覆盖回旧版。

## 问题二：buffer、Session 和输入传输分别由谁拥有？

`Session` 拥有 VM；`Repl<'a>` 借用同一个 `&'a mut Session`，拥有当前 source 与
`Option<CompileError>`。每次提交先编译整个 buffer；只有成功才执行那个 Proto 一次。
NeedMoreInput 保留 source，不执行前缀；Invalid 或 runtime 错误清当前 buffer，后面
的输入仍能读到同一 Session 的 globals，既有副作用不会回滚。

| 观察 | 保留或消费的状态 | 后续行为 |
| --- | --- | --- |
| NeedMoreInput | source + 原诊断 | 等下一行，不执行 |
| 编译并执行返回 | 清 source/pending | Values 或 runtime 错误，同一 Session |
| 普通编译/运行错误 | 清 source/pending | 报错后继续，同一 Session |
| `read_line -> Ok(0)` 且 idle | 无诊断 | 成功返回，CLI exit0 |
| `read_line -> Ok(0)` 且 pending | 清 source，take 诊断 | 返回原错误，CLI exit1 |
| `read_line -> Err` | 返回 Host 错误 | 不冒充 EOF，不换成旧 pending 诊断 |

执行成功或报错返回后才清当前输入。pending 保存最近一次未完成编译的诊断，后续
续行会替换它。公开 `submit` 表示按回车的完整物理行，参数不含末尾 LF；它会补 LF。
driver 内部 `submit_fragment` 接收实际是否
有 LF。无 LF 的末片段 `return 'broken` 先得到 NeedMoreInput，再在实际 EOF 返回
原诊断；有 LF 则是短字符串非法换行，报告后清输入，下一行 `=42` 是新 chunk。
完整无 LF `=42` 仍能运行。CRLF 的 CR 只有跟 LF 一起才被移除。shorthand 只在
idle 时剥一个 `=`，pending 内的 `=42` 和 idle 的 `==42` 都不能被“修好”。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol pending_chunks_execute_once_and_keep_the_same_session -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol ordinary_errors_clear_source_and_continue_without_rollback -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol expression_shorthand_is_only_rewritten_while_idle -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol unescaped_short_string_newline_is_an_error_and_does_not_swallow_next_line -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol input_error_is_not_converted_to_pending_eof -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol repl_output_keeps_binary_values_and_distinguishes_nil_from_no_values -- --exact
```

`run_repl` 的泛型 writer 接收提示符和返回值，Lua `print` 仍直接写进程 stdout，
不是全局输出拦截器。input Err 测试注入 `BufRead` 故障，不冒充真实控制台硬件故障。
当前 read_line 使用 UTF-8 String，非 UTF-8 输入属于 Host 错误。

## TODO：把原诊断移动一次

只修改 `starter.rs::finish(&mut PendingInput)`，不要改合同、solution、faults、
probes 或 ignore。这个教学投影由本地 compiler 生成真实 pending 诊断，但不暴露
Session/VM；真实 Repl 的 source/pending 仍由真实 controller 自己管理。hook 需要
清 source，并把 Option 中原诊断移动出来；练习约束禁止重新编译/执行输入。没有
Session 参数不等于 Rust 能禁止任何全局函数调用，合同另外观察真实 controller。

```diff
-let _ = input;
-Ok(())
+input.source.clear();
+match input.diagnostic.take() {
+    Some(error) => Err(error),
+    None => Ok(()),
+}
```

`take()` 让 Some 变为 None；`clone()` 只复制诊断，第二次 EOF 仍会返回同一错误。
silent fault 丢掉状态并返回成功，复现迁移前 pending EOF 直接 break 的问题。固定
clone fault 是两次调用的状态反例，真实 CLI 在第一次 EOF 返回后便结束。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_exercises migrated_eof_consumes_original_diagnostic_once -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference migrated_eof_consumes_original_diagnostic_once -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference historical_message_heuristic_misses_structured_kind -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference silent_eof_drops_original_diagnostic -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference cloned_eof_diagnostic_is_reported_twice -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference local_v100_controller_executes_once_and_recovers -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference local_v100_driver_distinguishes_final_fragment_from_newline -- --exact
```

初始 learner 预期失败；临时套用参考后通过，恢复原 TODO 后应再次失败。默认 workspace
test 中的 ignored 不表示练习完成。Release 为同样 run/test 加 `--release`。

## 问题三：怎样证明两个独立 binary 遵守同一合同？

两版各有本地 `tests/fixtures/repl-byte-contract.tsv`、`tests/repl_byte_contract.rs`
与 `tests/support/repl_byte_contract.rs`。仓库的身份检查要求这些文件逐 byte 相同；
不以一个 snapshot 的执行结果充当另一个的 expected。合同保存 stdin、argv、完整
stdout/stderr bytes 和退出码，只有明确的版本 banner 参数因快照而异，不做通用
文本 normalize。NUL/0xff、提示符、末尾 LF 和诊断次数都属于验收事实。

```powershell
py -3 tools/check_repl_byte_contracts.py
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --workspace --bins
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_byte_contract
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_byte_contract
```

进程合同先关闭 stdin，stdout/stderr 同时读取，超时后 kill/wait；两个流各自保持
顺序，不声称跨流有单一时间线。永久入口是提交的 corpus、harness、identity gate
和本 README，本机 target 下的日志只提供这次运行证据。这个合同证明所列 REPL
协议与边界；它不覆盖 TTY 编辑、signal、所有 Lua 官方 REPL 行为或增量 parser。

## 验收练习与检查点

1. 理解题：v0.18 已通过而 v1.0 尚未迁移时，共享 corpus 为什么仍有价值？指出
   compiler kind、controller 状态和 main adapter 各自必须修改的本地文件。
2. 实现题：只补 finish，要求 idle 成功、pending 原 kind/位置/message 返回一次、
   source/Option 清空。先验 learner，再验真实 controller 与两个 binary 合同。
3. 调试题：运行 faults，画出 silent 与 clone 两条路径的 source/Some/None；再把
   最后短串输入分别保留/去掉 LF，说明哪条路径返回退出码1、哪条恢复后退出0。

完成后应能沿 `main::repl -> run_repl -> Repl -> Session -> compile -> Vm` 指出
每个 owner，解释为什么 pending 不运行前缀、EOF 不重新编译、普通错误不重建 VM。
迁移会保留 v1.0 的独立构建边界；以后若协议变更，需要同时更新两端合同，并明确
哪些变化是版本差异，不用隐藏 normalize 掩盖行为分叉。
