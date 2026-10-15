# `string.find`：空匹配、字节 atom 与贪婪回退

先预测三组结果：`string.find('abc','',99,true)`、`string.find('acb','a.b')`、
`string.find('aaab','a*ab')`。它们分别暴露输入边界、pattern 解释与回退问题，不能
靠把同一个 `contains` 循环写得更长来解释。本包只有一个提出回退位置的窄 TODO。

从仓库根运行可执行反例与真实 VM 对照：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example string_find_problem_chain -- all
```

模式为 `faults|find|all`，缺省 all。faults5条、find8条、all13条不同的 `[ok]`；
示例只调用固定 faults/solution、真实 parser 和 VM，不读 starter，不启动嵌套 cargo。
包内9项参考、1项 ignored learner。binary 与官方 oracle 的完整 byte 合同另验，
本例 marker 不能代替真实 stdin/stdout/stderr/exit。

## 问题一：空 range 为什么不能当作没找到？

plain=true 明确选择字节字面查找；此时 `.` 只匹配点号。固定 ignore_plain fault
仍调用 pattern 路径，错误地把 `'abc'` 的第一个 byte 当成结果。init 是 Lua 的
1-based 起点，负数从末尾计算，超出两端后归到有效边界。过大的 init 不直接变成
nil：在 `'abc'` 的末端，空 pattern 仍成功，Rust 为 `Some(3..3)`，Lua 为 `(4,3)`。

```diff
-if start > subject.len() { return Ok(None); }
+let start = normalize_init(init, subject.len());
+// start may equal len: an empty match at that boundary is still success.
```

这里是边界流程 diff，真实 normalize 的负数/溢出处理请读本地模块，不能把这一行
当作函数全文。`Result<Option<Range<usize>>, FindError>` 有三层事实：Err 是非法/
不支持/资源错误；Ok(None) 是找不到；Ok(Some(start..end)) 是成功，start=end 表示
长度0。Rust 用0-based 半开 range，Lua 的返回值为 `(start+1,end)`；因此空 subject
的 `a*` 返回 `(1,0)`，并不矛盾，也不能拿 `range.is_empty()` 判断失败。

## 问题二：literal contains 为什么不能解释 `a.b` 与 `%d`？

plain=false 的 `a.b` 由三个 atom 组成，`.` 匹配一个 byte；`%d` 是数字 class，
`%.` 是转义的字面点号，`[A-C]` 是借用 pattern 切片的 set。固定 literal fault 在
`acb` 中找不到 bytes `a.b`，真实 matcher 却返回 `Some(0..3)`。这一版使用
`&[u8]`，不是 Unicode char 正则；NUL 与0xff也必须保留。

```diff
-subject.windows(pattern.len()).position(|bytes| bytes == pattern)
+let (atom, after_atom) = parse_atom(pattern, pattern_offset)?;
+// The matcher asks atom.matches(subject_byte), then continues at after_atom.
```

本包直接调用 `lua_vm::string_find::parse_atom` 和 `Atom::matches`，没有第二套
parser。真实 `Atom::{Literal,Any,Class,Set}` 中 Set 借用原 pattern，生命周期约束
它不能比源 bytes 活得更久；parse_atom 返回 atom 后的 cursor。重复量词由 matcher
读取那个位置，atom parser 不把整个 pattern 一次性编译成 AST。

## 问题三：贪尽后为什么还需要一组候选位置？

`a*ab` 对 `aaab`，第一个 atom `a*` 先消费三个 a，当前位置3的剩余部分只有 b，
后面的固定 `ab` 失败。把重复部分缩短一格，位置2的剩余 bytes 是 ab，整个匹配
成功。贪婪规定“先试长的”，不等于“只试最长的”。`a+ab` 的 plus 要保留至少
一个 a；`ab` 能匹配 `a*ab`，却不能靠回退到0次让 `a+ab` 成功。

```diff
-try_suffix(greedy_end)
+for candidate in descending_positions(greedy_end, start + minimum) {
+    if suffix_matches(candidate) { return success; }
+}
```

这是控制流 diff，真实 matcher 用有界的显式堆 stack 保存回退工作，不把这段伪代码
说成完整实现。练习 probe 用真实 parse_atom 得到 Literal(a)，用真实 atom.matches
扫描 greedy_end；只对当前固定反例的字面 suffix `ab` 调用 starts_with，展示3失败、
2成功，再对照真实 find 与 VM。这个固定 suffix 检查不是第二个 pattern matcher。

错误也按实际到达的位置发生：`a%` 的末尾 escape 非法，但 subject 为 b 时，首个 a
已经不匹配，不会到达 `%`，结果是 Ok(None)；subject 为 a 时才到达并返回 Err。
fixed eager fault 在匹配首个 atom 之前，直接调用真实 parse_atom 读取未触达的
suffix，错误地过早失败。不能为了“集中检查格式”先 eager parse 整个 pattern。
limit 错误也要返回 Err，不能吞成一次正常的没找到。

## TODO：消费一个候选位置，保留下一次回退状态

只修改 `starter.rs::next_position(&mut Retry)`；不要改合同、solution、faults、
probes 或 ignore。Retry 是贪婪阶段完成后的**候选位置投影**，不是 VM 的真实 stack。
它拥有 consumed range、minimum 与 Option<usize> next，不拥有 subject/pattern/VM。
每个 atom 在本包范围内消费一个 byte，所以 consumed 的长度也就是重复次数。

例如 consumed=0..3：star minimum=0，依次提出 Some(3)、Some(2)、Some(1)、
Some(0)、None；plus minimum=1，到 Some(1) 就结束。Some(0) 是有效候选，不能
拿0充当 None。每次 `take()` 消费上一个候选；再保存严格更小的候选，避免重复。
hook 只提出位置，后续 suffix 是否匹配由真实 matcher 决定。

```diff
-let _ = retry;
-None
+let position = retry.next.take()?;
+let lower = retry.consumed.start.checked_add(retry.minimum)?;
+if position < lower || position > retry.consumed.end { return None; }
+retry.next = position.checked_sub(1).filter(|previous| *previous >= lower);
+Some(position)
```

`&mut Retry` 是一次短借用，consume range 本身与 minimum 不变；next 是唯一推进的
状态。checked arithmetic 让极端 index 安全结束。参考合同还用固定调用上界让重复
同一位置的错误变成失败，不让测试死循环。生产 matcher 另有 steps/depth limits；
练习的 Vec 验收与极端 index guard 不代替生产的资源上限。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_exercises retry_positions_keep_greedy_order_and_the_repetition_minimum -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference retry_positions_keep_greedy_order_and_the_repetition_minimum -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference plain_and_large_init_need_distinct_boundaries -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference literal_contains_cannot_match_byte_atoms_and_escaped_classes -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference actual_atom_scanning_and_fixed_suffix_show_why_greedy_retries -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference plus_needs_one_byte_and_an_empty_match_is_still_some_range -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference an_unreached_malformed_suffix_is_not_an_eager_parse_error -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference actual_atom_parser_borrows_sets_and_matches_raw_bytes -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference a_limit_error_is_not_a_successful_no_match_result -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference actual_vm_maps_half_open_ranges_and_recovers_after_a_find_error -- --exact
```

每条 exact 恰选一项。初始 learner 失败；临时参考通过，恢复原字节并刷新 mtime 后
再失败，交付保留 TODO。Release 给同样 run/test 加 `--release`。

## 代码阅读与验收练习

阅读本地 `src/lib.rs::Session::execute_multi` → VM 的 StringFunction::Find adapter →
`string_find::find` 的 init/plain 分支 → parse_atom/Atom::matches → 显式回退 stack →
Range 映射与错误返回。先看 fixed faults 的失败事实，再读 real_retry 与 lazy_error
源码锚点，确认教学投影和真实 matcher 的边界。

1. 理解题：分别解释 None、Some(0..0)、Err；把 Some(3..3) 转成 Lua 两个位置，
   再说明 plain=true 的点号为什么不能走 Any atom。
2. 实现题：只补 next_position；star 必须含0候选，plus 最少1，候选降序且不重复。
   再跑真实 atom、延迟错误与 VM 合同，避免只通过状态投影。
3. 调试题：列出 a*ab/aaab 的 greedy_end、第一次与第二次 suffix 起点；比较
   `find('b','a%')` 与 `find('a','a%')` 的解析到达路径，解释 eager fault 的问题。

完成后应能指出两组输入 bytes 由谁借用、回退工作由谁持有、成功 range 怎样映射，
以及为什么一条普通 find 错误后原 Session 仍可执行。当前仍为 D005 partial：首包
推进 plain/init/empty、byte atoms/classes/escapes/sets 与贪婪 `*`/`+`，其它 pattern
能力另立任务；不以遇到未实现语法时的近似匹配冒充完整 Lua pattern，也不宣称全部
locale、captures、anchors 或任意复杂度行为已经兼容。
