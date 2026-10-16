# `string.find` captures：回退时还要恢复什么？

先预测：`string.find('aaab','(a*)ab')` 的 capture 是 `aaa` 还是 `aa`？把 pattern
换成 `()`，返回的是空字符串还是位置？如果后缀失败，只把 subject cursor 向前退
一格，已经关闭的 capture 会自动重新变成 Open 吗？本包沿这些问题练习状态恢复。

从仓库根运行固定设计反例与真实 VM 对照：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example string_find_captures_problem_chain -- all
```

模式为 `faults|captures|all`，缺省 all；faults3条、captures9条、all12条不同 `[ok]`。
示例运行固定 faults/solution 与真实 API，不读 starter，不启动嵌套 cargo。参考9项，
learner1项 ignored；binary/官方 oracle 的 raw byte 合同另验，本例 marker 不代替
完整 stdout/stderr/exit。上一版真实 binary 到达 capture 时返回 Unsupported；本包
truncate、clear、预算 rewind 都是**设计故障**，不是声称旧 binary 已有错误 capture。

## 问题一：成功 range 为什么还不够表达 capture？

整个匹配的 span 与 capture 列表是两层结果。`((a)b)` 的 capture 按左括号打开的
顺序保存：外层 `ab` 先于内层 `a`，不是按关闭顺序颠倒返回。工作槽位还要表达
尚未关闭的状态，不能只存一个 Vec<Range>：真实
`CaptureState::{Open(start),Closed(range),Position(offset)}` 把三种状态分开。
成功时转换成 `Capture::{Bytes(range),Position(offset)}`，不能把 Open 当成成功值。

空 capture 与位置 capture 也不是一回事。对 subject `b`，`(a*)` 成功捕获空 bytes，
VM 返回 `(1,0,'')`；`()` 捕获位置，返回 `(1,0,1)`。两者的整体 span 都是0..0，
第三个值分别是 String 与 Number。Rust offsets 为0-based；VM 将 Position 加1，
而 Bytes 通过 range 截取数据。不能用“range 长度为0”来猜 capture 的种类。

```diff
-struct Match { span: Range<usize> }
+struct FindMatch {
+    span: Range<usize>,
+    captures: Vec<Capture>,
+}
```

这段 diff 展示返回协议；真实公开类型还区分工作态 CaptureState 与结果态 Capture。
请读本地 `crates/lua_vm/src/string_find.rs`，本包直接使用这些真实类型，没有复制
一个缩小版 matcher 或自己的 capture 状态枚举。

## 问题二：只 truncate 为什么无法回退？

`(a*)(a)b` 在 `aaab` 上先让 a* 消费3个 a，随后 `)` 把第一个槽位从 Open(0)
改成 Closed(0..3)，再打开第二个 capture。第二个 a 在 b 处失败，重试2个 a 时，
不仅 subject 位置要回到2，旧槽还要恢复到 checkpoint 保存的 Open(0)，新增的第二
槽要删除。随后旧槽关闭为 Closed(0..2)，第二槽重新得到 Closed(2..3)，返回 aa,a。
仅截短 Vec 长度可以删新增槽，却修不好同一索引里已经变更的状态。反过来全部 clear
又会丢掉回退点之前仍应存在的 captures。更小的 `(a*)ab` 也保留在真实合同中，
但它不能单独代替这条同时改旧槽与添加新槽的反例。

```diff
-live_slots.truncate(saved_slots.len());
+live_slots.clear();
+live_slots.extend_from_slice(saved_slots);
```

真实 `capture_checkpoint` 克隆当前全部工作槽，返回拥有这些状态的 checkpoint；
`checkpoint.states()` 只借出只读 slice。保存之后 live Vec 可以继续变动；回退时
复制 checkpoint 内容，既修复旧槽又删除新增槽。每个后缀尝试与外层 candidate 都要
从适当状态开始；一次 find 的 captures 不留给下次 find。真实 runtime 在 Retry
处使用同一组 capture_checkpoint/restore_captures API，本包另作一致性对照。

## 问题三：哪些所有权要保留，哪些计数不能恢复？

checkpoint 拥有的是状态枚举和 range，不拥有输入 subject bytes。FindMatch 的
Bytes 也只是 range；VM 必须趁 subject 参数还在，将对应 bytes 复制成独立
ByteString，才把 Value 返回 Lua。probe 返回含 NUL/0xff 的 capture，执行后续 GC
和另一条 find，再丢掉 Session，仍检查此前 Value 的原 bytes；不经 UTF-8 或
to_string。这个所有权检查不把单纯的 Range 描述成已经拥有输入字符串。

回退工作与预算则属于不同 owner。回退要恢复逻辑 capture 状态，已经花掉的
matching steps 不能恢复；否则反复失败可以不断获得“免费重试”。真实 budget 不在
checkpoint 里，保存/恢复槽位本身也会计费。fixed work-counter fault 用一组明确
的教学整数模拟“8剩5又恢复8”的错误，不冒充真实 Budget 或历史 VM 算法；真实
find_captures_with_limits 与 VM Limit 错误另验：`(a*)aab`/aaaaab 需要两次回退，
相同 depth 上限下30 steps报 Limit，100 steps成功捕获 aaa，不能把每次恢复都当成
预算重新开始。VM 默认预算耗尽后仍能在同一 Session 执行下一条输入。

## TODO：完整替换工作槽，保持借来的 checkpoint 不变

只修改 `starter.rs::restore_slots(slots, saved)`，不要改合同、solution、faults、
probes 或 ignore。slots 是工作 Vec 的可变短借用，saved 是真实 checkpoint.states()
借出的 slice。hook 需要复制每个状态；移走 checkpoint 会让它无法再次恢复，借用
slice 也不能在 checkpoint 销毁后继续保存。CaptureState::clone 复制 offset/range，
不复制 subject bytes，也不表示克隆整个 VM。

```diff
-let _ = (slots, saved);
+slots.clear();
+slots.extend_from_slice(saved);
```

这是一个恢复投影，不能自行匹配/解析输入；不接受预算引用，也不把预算塞进 saved。
合同给出真实 Open/Closed/Position，包括改旧槽和添加槽的情况，要求全量还原、删
新增、保持 checkpoint 不变且可反复使用。故障中人工修改 Position 是为了验“完整
状态”合同，不声称真实 matcher 每次回退都会改动所有三种槽位。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_exercises checkpoint_restore_replaces_old_slots_and_removes_new_slots -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference checkpoint_restore_replaces_old_slots_and_removes_new_slots -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference truncate_or_clear_cannot_restore_mutated_existing_slots -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference restoring_a_work_counter_refunds_work_that_was_already_spent -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_backtracking_restores_capture_ends_before_retrying_the_suffix -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_nested_empty_and_position_captures_keep_distinct_kinds -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_candidate_and_next_find_start_with_independent_capture_slots -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_vm_returns_owned_binary_capture_values -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_capture_and_work_limits_return_errors_without_destroying_the_session -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_checkpoint_restore_matches_the_teaching_projection -- --exact
```

每条 exact 恰选一项。初始 learner 应失败，临时套解通过，恢复原 bytes 并刷新
mtime 后再次失败；交付保留 TODO。Release 给同样 run/test 加 `--release`。

## 代码阅读与验收练习

从本地 VM 的 StringFunction::Find adapter 读到 find_captures，再读 capture 状态、
checkpoint、Retry 的恢复和 finish_match 的结果转换，最后回到 adapter 的
ByteString/Number 创建。旧 find 的 Range 投影仍会验证 captures，不代表可以跳过
捕获语法错误。Limit、Invalid 与 Unsupported 仍通过 Result 保留各自失败意义。

1. 理解题：给 `(a*)ab`/aaab 画出 Open(0)→Closed(0..3)→恢复Open(0)→Closed(0..2)，
   说明为什么 slot 数量正确仍可能 capture 错误。
2. 实现题：只补 restore_slots，检查旧槽恢复、新槽删除、checkpoint 可重复使用；
   再跑真实 retry、nested/position、owned bytes 与预算验收。
3. 调试题：运行三个设计 faults，分别指出被遗漏或错误恢复的状态；将 `(a*)`
   与 `()` 的第三个返回值标注类型，并解释 range 本身为什么还不是拥有的字节值。

完成后应能区分 checkpoint、live capture slots、工作 budget 与返回 ByteString 的
owner；能解释一次 limit 或 malformed 错误后同一 Session 为什么仍可执行。
真实 API 最多32 captures，超限与过小工作预算返回 Err，不是正常 nil。当前仍为
D005 partial，backrefs、balanced/frontier、?/非贪婪-及其余 pattern/format 能力另立
工作包；本批不宣称完整 Lua pattern、全 locale 或官方复杂度策略等价。
