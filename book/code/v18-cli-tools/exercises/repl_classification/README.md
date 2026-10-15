# REPL 分类：诊断文本不承担控制流

前置：理解旧 repl_recovery 的 source/Session 生命周期。约 20 分钟，只完成一个
`decide` TODO。使用本轮实际修复的 v0.18 compiler；不实现缩小版 parser，不读取
`message` 猜测语法类别，也不改 parser、VM 或 source buffer。

## 先预测，再观察

下面两份输入都在末尾报错。下一条物理输入会先带来一个换行，它们都能续行吗？

```lua
return 40 +
```

```lua
return 1e
```

前者补 `2` 得到 42；后者补 `2` 仍然保留损坏的 `1e` token，不能把不同物理行
拼成 `1e2`。错误位置等于 source.len() 并不提供这层语法知识。

从仓库根运行，先构建同 profile 的真实 binary；示例不嵌套调用 Cargo：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --bins
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_classification_problem_chain -- keywords
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_classification_problem_chain -- eof
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_classification_problem_chain -- wording
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_classification_problem_chain -- real
```

默认 `all` 依次重放。固定 fault 与 solution 不导入 starter，修完练习后反例仍然
存在。`keywords` 复现旧三个 message 片段漏掉 `return 40 +`；`eof` 用真实词法错误
`return 1e`、`return 0x`、十进制 escape 超过255证明“错误点在 EOF”不够。

关键输出：

```text
before keywords: expression=Reject appended-chunk-compiles=true
after structured: expression=Preserve result=[Number(42.0)]
before EOF-position: bad-exponent=Preserve after=Reject appended-newline-does-not-repair=true
wording-only mutation: NeedMoreInput=Preserve Invalid=Reject keywords-false-positive=Preserve
```

`wording` 只修改两个真实 CompileError 的 message，保留 position/kind。它证明控制
决策不随诊断措辞变化；含“unterminated”的确定错误文本仍应 Reject。这是故意注入
的措辞变化，不声称真实 parser 当前恰好使用那段文字。

## 一个独立 TODO：只消费编译器的结构化协议

`compile(&[u8]) -> Result<Proto, CompileError>` 签名不变。Ok 拥有编译后的 Proto；
Err 附带 `CompileErrorKind::{Invalid, NeedMoreInput}`。`decide` 只共享借用结果，
返回三个无数据的动作，不 clone Proto、VM 或错误，也不执行代码。外围 REPL 保管
source 和 Session，负责按动作提交、保留或丢弃 source。

只改 [starter.rs](starter.rs) 的 `decide`：

```diff
 match compilation {
     Ok(_) => Decision::Execute,
-    Err(_) => Decision::Reject,
+    Err(error) if error.kind == CompileErrorKind::NeedMoreInput => Decision::Preserve,
+    Err(_) => Decision::Reject,
 }
```

[contracts.rs](contracts.rs) 直接调用实际 compiler。共享合同同时检查完整输入、
七种尚未完成输入和六种确定错误；七份 prefix 逐一追加 suffix 再编译成功，返回
40 + 2 的完整 Proto 还交给真实 VM 执行，得到 Number(42)。分类成功不等于执行，
`Preserve` 分支不能提前运行已有前缀。合同也将真实错误的 message 改写后重新调用
同一 hook，防止用越来越长的关键词清单替代结构化分类。

阅读顺序：先看 [compiler](../../crates/lua_compiler/src/lib.rs) 的 CompileErrorKind
与 compile，再看 [model.rs](model.rs) 的 Decide/Decision、[contracts.rs](contracts.rs)
的真实输入矩阵与 [solution.rs](solution.rs) 的借用匹配，最后对照
[real.rs](real.rs) 的实际进程协议；不要从 diagnostic 文本反推 parser 状态。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_classification_exercises -- --ignored --exact structured_errors_choose_the_next_repl_action
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_classification_reference
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_classification_reference an_error_at_eof_does_not_imply_that_a_new_line_can_repair_it -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_classification_reference diagnostic_wording_cannot_change_the_structured_decision -- --exact
```

初始 ignored 验收在 expression 的 Preserve 断言失败。完成后同一合同通过。
reference **7 项**分别验证共享合同、关键词漏判、EOF 位置误判、措辞独立、短字符串
与长 token、七组真实 Proto 执行，以及真实 binary。reference 通过不表示 TODO 完成。

## 分类来自哪里，能力又止于哪里

编译器先处理 token，再根据 parser 正在期待的语法项与实际 EOF 标记 incomplete；
REPL 不重新推断它。缺少 end、括号、table/function 末尾和表达式可需要更多输入；
long string/comment 可以包含换行，因此未闭合时可以继续。短引号字符串遇到未转义
的物理换行已经确定非法，应立即清 source 再接受下一条输入。

这里验收的是当前实现的明确分支与具体 prefix/suffix，不能推断“每个被标成
NeedMoreInput 的任意前缀，都存在某个合法后缀”。parser 的局部 EOF 期待不构成
完整的可扩展前缀判定器，后续整体错误也可能尚未被访问。普通 `compile` 仍以 Err
报告未完成脚本；NeedMoreInput 是给调用者的类别，不等于脚本已经编译成功。

`real` 跑四个实际 `lua_app -i` 子进程：表达式得到42且无 diagnostic、非法指数
与短字符串换行各立即报一次错误且随后 =42 成功、long comment 补闭合符后再求值42。
每个检查完整 stdout（含 banner 与 > / >> prompt）、退出码和 diagnostic 数量/内容。
测试使用 `CARGO_BIN_EXE_lua_app`；示例定位同 profile sibling；共享有界 runner
并发 drain、关闭 stdin 后等待。EOF 的最终 pending 诊断属于相邻 EOF 练习包。

Release 重放也先构建对应 binary：

```powershell
cargo build --release --manifest-path book/code/v18-cli-tools/Cargo.toml --bins
cargo test --release --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_classification_reference
cargo run --quiet --release --manifest-path book/code/v18-cli-tools/Cargo.toml --example repl_classification_problem_chain -- all
```

理解题：CompileError.message 为什么仍有价值，为什么不能决定 source 去留？
实现题：完成唯一 decide TODO，以 ignored 合同验收。调试题：将“在 EOF 报错都
Preserve”套到 `return 1e`，指出哪一层最先知道数字 token 已经非法。

完成检查：能分别解释三个动作；按真实 kind 判断短/长字符串；用措辞改写反例证明
控制协议稳定；能运行真实 binary；能陈述局部分类的边界。迁移时把 kind 消费留在
REPL，fixed faults 与 observations 留在教学包，旧启发式材料保留其历史版本归属。
