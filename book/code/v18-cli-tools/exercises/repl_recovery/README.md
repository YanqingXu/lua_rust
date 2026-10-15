# REPL：保留 Session，提交或丢弃当前 source

前置：已理解 `Session` 为多个输入保存同一个 VM。约 20 分钟，完成一个
`submit` TODO：编译尚未完成时保留 source，成功或普通错误后清空它，再接受下一行。
本包使用真实 Session/编译器，不改生产 REPL 或编译器，不扩展完整续行语法。

## 先预测，再重放固定反例

逐行输入下面代码，最后的 saved 应是什么？如果每行执行后都清空 source 呢？

```lua
saved=40
if false then
saved=99
end
=saved
```

从仓库根运行。`real` 和默认 `all` 都需要同 profile 的真实 binary；example
只定位 sibling binary，不在运行中启动 Cargo，也不探测全局安装的 Lua。

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --bins
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_recovery_problem_chain -- continuation
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_recovery_problem_chain -- errors
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_recovery_problem_chain -- early-return
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_recovery_problem_chain -- heuristic
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_recovery_problem_chain -- real
```

可以改为 `all`（默认）一次重放。CLI 固定比较 `faults` 和 `solution`，**不调用
starter**；修完 TODO 后仍展示同样的 before 故障。它们是局部人为错误设计，不是
声称历史版本曾有这些实现。

`continuation` 的完整固定输出：

```text
before: clear every line
  line="saved=40"; values: []; buffer=""
  line="if false then"; incomplete: expected 'end' to close if; buffer=""
  line="saved=99"; values: []; buffer=""
  line="end"; compile error: expected local, assignment, do block or return; buffer=""
  line="=saved"; values: [Number(99.0)]; buffer=""
after: preserve incomplete source
  line="saved=40"; values: []; buffer=""
  line="if false then"; incomplete: expected 'end' to close if; buffer="if false then\n"
  line="saved=99"; incomplete: expected 'end' to close if; buffer="if false then\nsaved=99\n"
  line="end"; values: []; buffer=""
  line="=saved"; values: [Number(40.0)]; buffer=""
```

错误版本丢掉 if 的前缀，下一行 saved=99 变成独立 chunk，实际执行了本不该执行
的分支。健康版直到 end 才编译/执行完整 chunk，saved 保持 40。

`errors` 重放另一个问题：语法错误留在 buffer，会使后续正确输入继续编译失败；
运行错误留在 buffer，会使已经发生的副作用再次发生。关键输出为：

```text
  direct Session observation outside polluted buffer: effects=[Number(2.0)]
after: runtime error clears source but preserves effects
  line="effects=0"; values: []; buffer=""
  line="effects=effects+1;error('recover')"; error: line 1 (pc 8): recover; buffer=""
  line="return effects"; values: [Number(1.0)]; buffer=""
```

其中 direct Session observation 是宿主绕过被污染的 source，实读**同一个 Session**
中的 effects，以观察两次执行；它不是错误版 REPL 已成功处理了下一行。

`early-return` 固定将普通错误转换为 Err 后通过 `?` 传播出循环：

```text
before: early return stopped=true submitted=2 saved=[Number(40.0)]
```

后两条输入没有执行。健康循环继续处理 saved=42，再求值 =saved，实际得到 42。
这项控制流对照独立于 buffer TODO，修完 buffer 不会自动修复外围的 `?`。

## 一道独立 TODO

框架 `Repl` 只创建一次 Session；hook 每次仅可变借用它和当前 source。模型的
`append_line` 固定对应真实 `repl` 的入行协议：

- source 为空且空白行：忽略，不执行；
- source 为空且 trim_start 后以 `=` 开头：改写为 return；
- source 非空：原样追加输入，空行和 `=` 也如此；
- 每次追加一个 `\n`，再交给真实 Session。

只改 [starter.rs](starter.rs) 的 `submit`，保存本次 Event 并决定 source 的去留。
`Event::Incomplete` 是本实验对编译错误的包装，生产 `AppError` 没有这个 variant。
框架不重建 Session；它也不捕获进程 stdout，结果显示由真实 binary 对照。

```diff
 let event = attempt(session, source);
-source.clear();
+if !matches!(event, Event::Incomplete(_)) {
+    source.clear();
+}
 event
```

[solution.rs](solution.rs) 用 match 写出相同决策。共享
[contracts.rs](contracts.rs) 通过相同 hook 检查：未完成与 pending 空行保留、成功
清空、普通编译/运行错误清空后继续、原 global 和运行副作用仍可读、仅首行的 `=`
改写，以及 `return nil` 和无返回值的区别。只改 source，不复制 VM/global/cache。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_recovery_exercises -- --ignored --exact source_lifecycle_recovers_after_errors
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_recovery_reference
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_recovery_reference faulty_retained_errors_pollute_next_input_and_repeat_runtime_effects -- --exact
```

初始 ignored 验收应失败：输入 if true then 后，source 实际为空，预期保留
`"if true then\n"`。完成后同一合同通过。默认 reference 有 **7 项**：共享合同、
三种固定故障、真实启发式漏判，以及两组真实 process 对照。reference 成功或 CLI
退出 0 都不等于 starter 完成，以 ignored 验收为准。

## 错误恢复没有撤销 Session 的副作用

编译失败时整个 chunk 尚未执行；runtime error 之前的 global 修改、输出和文件
操作可能已经发生。本包失败前 effects 加一，下一输入仍读到 1；保留旧错误源码
则重复执行成 2。清空 source 只是丢弃当前待提交文本，不提供事务回滚，也不证明
所有 VM/GC 状态在所有错误分支均已恢复。

当前多结果接口区分 `return nil` 的 `[Nil]` 和 `return` 的空 Vec。生产
`print_values` 对空 Vec 不打印结果行，对单个 Nil 打印 `nil\n`。模型保留真实
Vec，真实 process 检查完整 stdout，避免只检查最后一个数字。

## 启发式、EOF 和管道的真实边界

生产 `is_incomplete` 只看 `CompileError.message.contains(...)`，匹配
`expected 'end'`、`unexpected end`、`unterminated` 三个片段。模型复制这三个条件，
没有增加结构化错误类别。直接使用当前编译器，可以复现确定的**漏判**：

```text
heuristic: message="expected an expression" incomplete=false appended_chunk_compiles=true ordinary_error=true buffer_empty=true
```

`return 40 +\n` 编译失败，但追加 `2\n` 后可编译成返回 42 的程序。当前 REPL
却将前一行当普通错误清空，下一行 2 又成为独立语法错误，最后 =42 才恢复。
本题保留并验收这个现有边界，不把字符串判断包装成完整 Lua 续行 parser，也
没有声称已复现所有种类的误判。若未来修编译器，应另订 NeedMoreInput 合同。

`real.rs` 通过共享有界 process runner 跑 **8 个真实子进程**。集成测试使用
`env!("CARGO_BIN_EXE_lua_app")`；example 使用同 profile sibling。关键结果：

| 实际输入模式 | 完整检查的现象 | code |
|---|---|---:|
| `-i`，先语法错再运行错 | 继续打印 42、effects=1、原 saved=40；两条 diagnostic | 0 |
| `-i`，return nil / return / =42 | stdout 含一次 nil 行；空 Vec 没有结果行 | 0 |
| `-e "error('recover')"` | 无 stdout，一条 recover diagnostic | 1 |
| `-i`，未闭合 if 后 EOF | pending 不重试、不报告错误，`never` 不执行 | 0 |
| 无 `-i` 的 pipe，运行错误 | 当一个 chunk 执行，只打印 first，不执行 after | 1 |
| 无 `-i` 的 pipe，后行语法错 | 整段先编译，前行 print 也不执行 | 1 |
| 同一段语法错输入加 `-i` | 前行已完整 chunk 先打印，后行报错后继续到 EOF | 0 |
| `-i`，上述 return 40 + 漏判 | 两条编译 diagnostic，最后 =42 仍可执行 | 0 |

EOF 分支只是 break；pending source 随 `repl` 局部变量析构而丢弃，没有额外
`source.clear()` 或最终编译。普通 Lua 输入错误在循环里处理；stdin 读取或
stdout 写入等宿主错误仍能离开循环。`-i` 即使接到管道也有 banner/prompts；
无参数且 stdin 非终端则 read_to_end 读取一个脚本，不走 REPL。

真实 `real` 输出逐项列出 name/code/完整 stdout 和 diagnostic 数量；内部还核对
诊断具体内容。Release 重放前也要先构建对应 binaries：

```powershell
cargo build --release --manifest-path book/code/v18-cli-tools/Cargo.toml --bins
cargo run --quiet --release --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_recovery_problem_chain -- real
```

理解题：为什么清 source 不应重建 Session，为什么 runtime error 之后 effects
仍是 1？调试题：为什么缺少 `end` 能进入续行，return 40 + 却不能？指出判断依据
在哪个字段，不能仅从行末字符猜测。

完成检查：能预测三个 fault 的实际状态；独立完成 submit；解释 source/Session
两种生命周期、空 Vec/Nil、EOF/pipe 的区别；指出启发式的已验证漏判。迁移时只将
source 决策留在生产 REPL，观察 Event、故障 hooks 和子进程对照保留在教学包。
