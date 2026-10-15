# EOF 到来时，未完成的 chunk 应该去哪里？

本包只有一道 TODO：`starter.rs::finish(&mut PendingInput)`。先预测：`return 40 +`
后关闭 stdin，应该成功退出、继续显示提示符，还是报告一次原编译诊断后结束？已经完成
的 `saved=40` 会不会因为后续 pending EOF 而回滚？EOF 是输入结束，不能再等待下一行。

从仓库根先构建同 profile 的真实 CLI，再运行固定对照：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_eof_problem_chain -- all
```

模式为 `faults|eof|real|all`，缺省 all。示例使用固定 faults/solution，**不读取 starter**。
`all` 输出 15 条不同的 `[ok]`：两个固定故障、三个参考合同要点、十个真实 CLI case。
包内参考 5 项；额外 `tests/repl_input_protocol.rs` 有 6 项实际 controller/I/O 测试。
learner 是 1 项 ignored，默认通过不能替代显式 TODO 验收。

## 问题一：诊断消息为什么不能决定是否继续读？

编译器保留 `compile(&[u8]) -> Result<Proto, CompileError>`；错误新增结构字段
`CompileErrorKind::{Invalid, NeedMoreInput}`。`return 40 +\n` 的错误消息仍是
`expected an expression`，但缺的是 EOF 后可以补齐的表达式，kind 是 NeedMoreInput。
`local =\n` 已经看到错误 token，kind 是 Invalid；后面加多少行也不能移走这个 `=`。
`if true then )` 的诊断即使提到期望的 `end`，也不能据此保留错误输入。

短字符串遇到未转义物理换行是 Invalid：输入 `return 'broken` 后按回车，下一行
`=42` 必须作为新 chunk 执行。若最后一片输入为无 LF 的 `return 'broken`，则实际
遇到的是 EOF，诊断为 NeedMoreInput；driver 不合成换行，随即由 pending EOF 报错
退出1。同样的 bytes 后带 LF 则是 Invalid，普通错误报告后 idle EOF 退出0。
长字符串 `return [[unfinished` 则可以在后续输入中
用 `]]` 关闭。这些事实来自 lexer/parser 的状态，不能由 `contains("unterminated")`
或诊断措辞推出。分类练习见相邻 `../repl_classification/` 包。

## 问题二：谁保存 source，谁拥有 VM？

真实 `src/lib.rs::Repl<'a>` 长期保存 source 与待报告诊断，临时借用同一个
`&'a mut Session`；Session 拥有 VM。每次 submit 只调用一次 execute_multi，内部
编译一次，成功后直接执行得到的 Proto。NeedMoreInput 不会运行已经能看懂的前缀；
后续补齐后整个 chunk 执行恰一次，不能先 execute 试探、再 execute 真正运行。

| 输入结果 | source/诊断 | VM 与下一行 |
| --- | --- | --- |
| Blank，且当前 idle | 保持空 | 等待输入 |
| NeedMoreInput | 保留 source，更新原诊断 | 不执行，下一行追加 |
| Complete | 清空 source/诊断 | 执行一次，保留同一 Session |
| Invalid 或运行错误 | 清空 source/诊断 | 报错后继续；已发生运行副作用不回滚 |
| idle EOF | 保持空 | 返回成功，CLI exit0 |
| pending EOF | 消费诊断、清空 source | 返回原错误，CLI 一次 stderr、exit1 |
| input I/O Err | 返回宿主错误 | 退出1，不能冒充 EOF |

`=expr` 只在 idle 时改写为 `return expr`，而且只移除第一个 `=`。pending 中的
`=42` 是普通 Lua token，不能为了“帮助续行”重写；`==42` 也不能变成合法 `return 42`。
本轮将旧 `trim_start_matches('=')` 改为 `strip_prefix('=')`，明确 shorthand 只有
一个前缀；`expression_shorthand_is_only_rewritten_while_idle` 验证这个输入协议边界。

## TODO：只消费诊断，不再次编译或执行

练习 `PendingInput` 只是 EOF 所需的 source 与 `Option<CompileError>` 投影，不拥有
Session。hook 获得 `&mut` 短借用，可以清空 source，并用 `take()` 把诊断从 Some
移动出来，同时留下 None；它没有编译器调用或 VM 引用。`clone()` 复制诊断但不改变
所有权，重复 finish 仍会报告同一错误；`take()` 把“最多报告一次”编码进状态。

```diff
-let _ = input;
-Ok(())
+input.source.clear();
+match input.diagnostic.take() {
+    Some(error) => Err(error),
+    None => Ok(()),
+}
```

只修改 `starter.rs::finish`。不要改合同、solution、faults、real 或 learner ignore。
参考与 learner 调用同一 `contracts.rs`，验证 idle、未完成表达式、block、长字符串，
必须原样保留 kind/位置/message，并在第一次 EOF 后清空 source/诊断，第二次返回 Ok。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_eof_exercises eof_reports_pending_once_and_releases_buffer -- --ignored --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_eof_reference eof_reports_pending_once_and_releases_buffer -- --exact
```

初始 learner 预期失败于 pending EOF 没有返回原错误；补全后相同合同通过。原 starter
应保留 TODO，参考验收提供永久复现入口；本机验证的临时套解不作为交付答案。

## 两个固定反例与一次真实 EOF

`discard_pending` 清空并返回 Ok，错误 source 消失却被当作成功；这正是旧 CLI 在
pending EOF 时直接 break 的问题。`clone_without_consuming` 返回原诊断的克隆，但
source/Some 都还在：两次调用都返回同一错误。后者是窄 hook 的状态反例；真实 CLI
遇到 EOF 后只调用一次 finish 并立即结束，不把测试两次调用说成实际死循环。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_eof_reference faulty_discard_pending_reports_success_and_loses_original_diagnostic -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_eof_reference faulty_clone_without_consuming_repeats_diagnostic_and_keeps_source -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_eof_reference actual_repl_eof_does_not_execute_prefix_and_consumes_diagnostic_once -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_eof_reference real_cli_distinguishes_idle_pending_invalid_and_final_line_eof -- --exact
```

真实 EOF 合同先初始化 `effects=0`，再提交 `effects=effects+1; if true then`。
NeedMoreInput 和 finish_eof 都不能执行前缀，之后同一 Session 中 effects 仍为0。
真实 CLI 十 case 精确匹配 stdout bytes、退出码与诊断次数：idle、空白后 EOF、最后
一行没有 LF 的完整表达式、无 LF 未闭短串、pending 表达式、带 print 前缀的 pending block、先完成再 pending、
普通错误恢复、短字符串换行恢复、pending 长字符串。提示符与诊断属于两个独立流，
不宣称二者有跨流时间顺序。共享 runner 关闭 stdin，子进程读到的是实际 EOF。

`src/lib.rs::run_repl` 泛型接收 BufRead/Write，区分 read_line 的 Ok(0) 与 Err；
read_line 返回非空末片段却没有 LF 时，只追加实际 bytes。公开 submit 仍表示完整
物理行（按回车），内部 submit_fragment 才接收是否确有换行的事实。
下列测试在一段 pending 输入后注入 I/O Err，要求返回宿主错误、没有 pending EOF
诊断且 effects 仍为0；这不是在真实控制台上复现硬件 I/O 故障。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_input_protocol input_error_is_not_converted_to_pending_eof -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_input_protocol pending_chunks_execute_once_and_keep_the_same_session -- --exact
```

Release 也先 `cargo build ... --workspace --bins --release`，再为上述 run/test 加
`--release`；示例从 current_exe 寻找同 profile sibling，不启动嵌套 cargo。

## 验收练习与检查点

1. 理解题：给出 `return 40 +`、`local =`、未转义短字符串换行的状态，以及后续
   `=42` 属于追加还是新 chunk。解释“运行错误清 buffer”为什么不表示回滚 globals。
2. 实现题：只补 finish，执行显式 learner 与 reference 合同。成功条件是原诊断一次、
   source/Option 清空、idle 成功；不能通过把 pending 统一归为成功来满足清理。
3. 调试题：运行 faults 后，为 clone 反例画出两次 EOF 前后的 Some/None 与 source。
   再删掉临时答案，确认 learner 重新失败，fixed CLI 与参考仍通过。

完成后应能沿 `main::repl -> run_repl -> Repl::submit -> Session::execute_multi -> compile`
指出每个 owner；沿 `Ok(0) -> finish_eof -> pending.take -> main ExitCode1` 指出为什么
不会再执行 Lua。文件/-e/非-i整段 stdin 仍采用普通 compile 错误退出，只有 REPL
需要保留 NeedMoreInput；未声明新的 Lua 语法或完整官方 REPL 等价性。
