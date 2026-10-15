# `string.format`：从字符串拼接走到格式协议

先预测：`string.format('%f',3.5)` 应该返回 `3.5` 还是 `3.500000`？把格式换成
`%08.2f` 后，读取 `%` 后一个 byte 能知道输出吗？`%%:%s:%d` 的三个 `%` 是否
消耗三个参数？本包沿这三个问题读真实 parser/VM，只留一个参数游标 TODO。

从仓库根运行可执行反例：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example string_format_problem_chain -- all
```

模式为 `faults|format|all`，缺省 all。faults4条、format8条、all12条不同 `[ok]`；
示例调用固定 faults/solution 与本快照真实 compiler/VM，不读取 starter、不启动
嵌套 cargo。参考9项，learner1项 ignored；默认 workspace 通过不表示 TODO 已完成。
真实 binary 与官方 oracle 的字节合同另有集成验收，本例 marker 不代替 raw streams。

## 问题一：`to_string()` 为什么不能实现 `%f`？

Rust `3.5_f64.to_string()` 选择简短显示，Lua `%f` 的缺省 precision 是6。指定
`%.2f` 又要求两位，而不是“去掉多余零”。数值、精度、符号、填充是不同输入，不能
把 Value 的通用显示直接当作格式化协议。固定 fault 实际返回 `3.5`；同一个测试
调用本地 VM 得到 `3.500000` 和 `3.50`，这不是只比较两个教学模型。

```diff
-output.extend(number.to_string().as_bytes());
+let precision = usize::from(spec.precision.unwrap_or(6));
+let digits = format!("{:.*}", precision, number.abs()).into_bytes();
+// Sign and width padding are applied separately by the real renderer.
```

这段 diff 只展示 precision 的最小变化；完整 renderer 还处理符号、`#` 保留小数点、
零填充与左对齐优先级。请沿本地 `crates/lua_vm/src/string_format.rs::render_number`
阅读真实实现，不把这三行说成完整 Lua/C 数字格式实现。

## 问题二：为什么 `%` 后需要一个游标解析器？

`x%08.2f!` 中 `%` 后的 `0` 是 flag；`8` 是 width；`.` 后 `2` 是 precision；
`f` 才是 conversion。固定单 byte fault 返回 `(b'0',3)`；真实
`parse_spec(format,2)` 返回 zero=true、width=8、precision=2、conversion=f，next=7，
指向 `!`。next 由返回值交给 dispatcher，避免重新猜测已经消费多少输入。

```diff
-let specifier = format[cursor];
-cursor += 1;
+let (spec, next) = parse_spec(format, cursor)?;
+cursor = next;
```

公开 `FormatSpec` 与 `Flags` 是真实 VM 使用的类型；本包没有复制一个缩小版 parser。
`parse_spec` 的 start 必须紧接 `%`，返回 next 紧接 conversion。flags 为
`-+ #0`，width 与 precision 最多两位；`.` 后没有数字表示 precision=0。Lua 5.1
按 flag 字符数检查上限，`%++d` 并非一概非法，六个 flag 字符才超界。数字过长、
缺 conversion、未知 conversion 都通过 Result 返回；literal `%%` 由 dispatcher
预先识别，不伪造 `FormatSpec { conversion: b'%' }`。

## 问题三：为什么不能把每个参数都 `Stringify`？

`%s` 与 `%q` 接受 string 或 number，拒绝 table、boolean、nil；`%d/%i/%f`
接受 number 或可转换数字字符串。宽泛的 Value 显示能给 table 产生可读文字，却不
代表 Lua `%s` 接受该 table。固定 fault 只检查 table 显示非空，地址文字并不作为
可复现 expected；真实 VM 必须返回类型错误。`%q` 也接受 number，不能错写成只收
string。格式串本身仍有其类型检查，不能因为后续转换而跳过第一参数。

`%%` 输出一个 `%`，不消耗参数。因此 `%%:%s:%d` 与 `('ok',7)` 对齐为 `%:ok:7`；
`string.format('%%')` 无需第二参数，而 `string.format('%%:%s')` 要报告缺参。
缺参、类型错误与非法格式是 Result 的不同失败边界；本包不依赖错误文本反推种类。

```diff
-output.extend(stringify(value));
-argument += 1;
+let value = required(argument)?;
+// The real dispatcher selects string or number validation from spec.conversion.
+let rendered = render(spec, value)?;
+output.extend(rendered);
+argument += 1;
```

这是流程 diff，`required/render` 在真实模块中有各自的具体实现。若先在局部 Vec
累积了 `prefix:`，后续参数错误仍使整次 format 返回 Err，没有成功的部分字符串；
这不是 Lua 的事务回滚，也不撤销此前 chunk 的 globals 或 I/O。

## TODO：参数游标什么时候才可以提交？

只修改 `starter.rs::consume`，不要改合同、solution、faults、probes 或 ignore。
输入 spec 来自真实 `parse_spec`；`values` 借用完整 builtin 参数切片，index0 是格式
字符串，cursor 从1开始。借用不转移原参数容器的所有权；成功返回一个 Value clone
并推进一格。Value clone 不是对 Lua table/heap 的深拷贝。缺参或类型错误返回
HostError，cursor 和借用值保持原样，便于观察“验证后才提交”的局部不变量。

本 hook 是**参数类型、缺参与游标提交的教学投影**。它复用已有
`lua_stdlib::Args::{required,bytes,number}`；这不替代真实 formatter 的数字范围、
字符串转数字边缘规则、精度与输出算法。合同使用有限的普通数值/十进制字符串，
完整转换和错误仍由真实 VM probes 与字节合同验收。%% 不进入这个 hook。

```diff
-let _ = (spec, values, cursor);
-Err(HostError::message("implement the argument cursor TODO"))
+let args = Args::new(values);
+let value = args.required(*cursor)?;
+match spec.conversion {
+    b's' | b'q' => { args.bytes(*cursor)?; }
+    b'd' | b'i' | b'f' => { args.number(*cursor)?; }
+    _ => return Err(HostError::message("unsupported conversion in exercise")),
+}
+let accepted = value.clone();
+*cursor += 1;
+Ok(accepted)
```

`?` 在验证失败时立即返回，成功路径末尾才变更 cursor。fixed eager fault 在检查
之前推进，类型错误和缺参都会跳过一个值；这条故障是教学游标的可运行反例，不声称
旧真实 VM 在一个已经返回 Err 的调用之后还会恢复并复用该游标。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_exercises argument_cursor_commits_only_after_validation -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference argument_cursor_commits_only_after_validation -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference to_string_does_not_implement_default_float_precision -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference one_byte_specifier_loses_flags_width_and_precision -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference stringify_accepts_a_table_that_percent_s_must_reject -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference advancing_before_validation_commits_a_failed_argument -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference actual_percent_escape_does_not_consume_an_argument -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference actual_parser_checks_the_whole_bounded_specification -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference actual_vm_applies_padding_precision_and_existing_conversions -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference actual_failure_returns_no_partial_string_and_session_remains_usable -- --exact
```

每条 exact 恰选一项。初始 learner 应失败；临时套用参考后通过，恢复原字节并刷新
mtime 后再次失败，交付保留 TODO。Release 给同样 run/test 加 `--release`。

## 代码阅读与验收练习

阅读顺序为本地 `src/lib.rs::Session::execute_multi` → compiler 生成 Proto →
`crates/lua_vm/src/lib.rs` 的 `StringFunction::Format` adapter →
`string_format::format` dispatcher → `parse_spec` → 各 renderer；从内层 FormatError
回到 VmError、AppError 和 CLI。练习只借真实 spec 与 Args 的窄接口，不再写 parser。

1. 理解题：对 `x%08.2f!` 标出字节位置、五段含义与 next；说明 `%f` 的6位缺省
   precision 为什么不是 Value 的通用显示规则。
2. 实现题：只补 consume，验成功只前移一次，缺参/类型错误不前移，原借用值仍在；
   再跑真实 percent、parser 和 VM 验收，避免只通过教学投影。
3. 调试题：运行 faults，画出 `%s` 收到 table、`%%:%s` 缺参、参数先推进后失败的
   状态；分别说明哪一步属于 parser、dispatcher、类型检查与局部 cursor 提交。

完成后应能解释为什么两条游标（format bytes、arguments）独立前进，为什么解析
错误不返回成功部分字符串，以及为什么普通 format 错误后同一 Session 仍可使用。
当前 D005 仍标 partial：只推进既有 `s/d/i/f/q/%%` 与这里列出的 flag/width/precision，
不扩展 `%x/%e/%g/%c` 全集，不宣称覆盖所有 C locale、非有限值、平台整数范围及
任意格式组合。后续兼容任务应继续由具体失败 fixture 与官方 oracle 推动。
