# string.find 的字节与捕获官方合同

从仓库根运行；Release 在 `--` 前加 `--release`：

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract real_binary_matches_seventy_five_fixed_find_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract fixed_find_corpus_preserves_seventy_five_arities_and_byte_coordinates -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract find_errors_keep_argument_categories_and_only_validate_reached_atoms -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract plain_truthiness_init_empty_ranges_and_c_prefixes_remain_distinct -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract unsupported_constructs_are_explicit_partial_errors_in_the_candidate -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract candidate_work_limits_are_errors_and_leave_the_session_usable -- --exact
```

固定文件是 `tests/fixtures/string-find/cases.tsv` 与独立 `driver.lua`。75条预期先手写，
再对实际 Lua 5.1.5 核对；driver 不读取 TSV，不用 candidate 输出反生成 expected。
四列为 id/result/arity/values：成功值用 `n:4,n:3` 或 `nil`，错误的后两列明确用 `-`，
没有隐含空字段。完整 stdout 必须为 `ok|2|n:4|n:3|after`、`ok|1|nil|after` 等，
或 `error|after`；每项 exit0、stderr 空。nil 的一个返回值与空匹配的两个坐标不同。
Lua 用 `select('#',...)` 保留元数，用算术/char 编码数字，after 在 pcall 完成后输出；
未知 id 的 assert 在 pcall 之外，driver 编译或选项错误不能当作成功错误场景。

这是 Lua 层协议，捕获后不 trim、不解码、不作 CRLF normalize。输入用 Lua char
构造 NUL/0xff；所有输出是无 LF 的 ASCII，Windows C stdout 文本模式不会改变坐标
协议。Rust runner 并发 drain stdout/stderr，关闭 stdin，5秒 deadline 后 kill/wait、
join；Python用 communicate 并在 timeout 后 kill 再 communicate/wait。输入输出
有界、无后代，不把该 runner 当作任意进程树管理器。

先做 schema 与 driver case ID 检查：

```powershell
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --check
```

它明确打印 candidate/official NOT_RUN，仅表示格式检查。实际比较强制同时给两个
binary 与固定源码 tar；缺参数或错归档 hash 会失败。以下适用于本仓库 Windows
x86_64 target 与已构建的官方 Lua，target-dir/triple 不同时据实际产物改路径：

```powershell
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --candidate-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --official-bin target/oracles/lua-5.1.5/build/Release/lua51.exe --official-archive target/oracles/lua-5.1.5/lua-5.1.5.tar.gz --output target/teaching-v100-find-2026-10-15/official-contract
```

参照 [Lua 5.1.5 source](https://www.lua.org/ftp/lua-5.1.5.tar.gz)，归档 SHA-256 是
`2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333`。源码
`src/lstrlib.c` 的 str_find_aux/classend/singlematch/max_expand 决定分流、位置、
原子与贪婪回退。首次官方 binary SHA-256 是
`47fd81e426c4f598e9fe2fdf28a0b36df721984ecfd66c8991eeef763693978d`，PE machine
0x8664（x64）；每次记录实际 binary/version/platform/hash，别的平台编译不要求
同一个 binary hash。source tar 身份与 binary 身份分别记录，版本字符串本身不证明
可复现构建。字符类验收按普通 C locale 的 ASCII 规则，不把 locale 全集外推为已验。

75条覆盖 plain 的 truthiness（0和空字符串也 true）、正负/零/超大 init、nil与空命中、
dot/escape/ASCII class/集合及补集/高字节范围、首尾锚与 `*`/`+` 后缀失败回退。
`%w` 不包含 `_`；5.1 没有 `%g` 类，未知 `%g/%G` 分别按 literal g/G。
`[]` 缺少后续 closing `]`，而 `[]]`、`[%]]` 能匹配 literal `]`。
错误验证是按匹配路径发生：`z[` 在 `abc` 上无命中，`a[` 到达坏集合才报 malformed。

subject NUL 与 pattern NUL 分开验：主体始终 length-aware；plain 分支按完整 pattern
bytes 搜索。默认分流只扫描 pattern 的 C-prefix，触发模式后 matcher 也在 pattern
NUL 结束，所以 `'.'..char(0)..'x'` 与 `char(0)..'.'` 有不同路径，不是统一截断输入。
Lua 5.1 将超大 init 截到末端，`find('abc','',99)` 为4,3；不能套用别的版本规则。

pcall false 只冻结跨引擎的稳定错误标志，不伪造 traceback/路径/诊断文字一致。
本地错误测试另外逐类核 VM Type、参数号、malformed 分类和可达性时机。候选独有的
Unsupported/Limit 由另外两个测试验证：`?`/`-`量词、backrefs、`%b/%f` 到达时
显式拒绝；step/retry-depth 耗尽返回 Limit，不冒充 nil，Session 后续仍可用。
这些不是与官方正常支持行为相等的75条之一；plain 路径不使用 pattern budget，
也不宣称整个VM/进程有统一资源限制。gmatch/gsub、backrefs、locale等继续 D005 partial。

每次运行创建唯一目录，保存双方各75项实际/预期双流、stdin、metadata，汇总另存
fixture/driver/checker/source成员/binary SHA与版本原流。错误/超时也归档，拒绝覆盖；
写入失败不报通过，不宣称 fsync/断电原子性。新证据不覆盖旧 format/REPL 合同，
也不冒充旧五项语言双 oracle 基线的新结果。

## 捕获增量：54条独立固定合同

旧75条fixture与driver保持原bytes，SHA-256分别为
`a14bf24da1b30e064f8e6bcc66d755decae537d3f28730d901ac55f22be2780d`与
`b82485d108683dd9e13ada899d5e82670289b4a6e7185a5b243ffde67e899153`。
新增 `tests/fixtures/string-find-captures/{cases.tsv,driver.lua}` 独立保存54项预期，
均先手写再核官方5.1.5，不改旧case去迁就新实现。两套合计129个不同场景。

捕获driver保留元数和类型：`n:3`是位置数字，`s:0:`是空字符串，
`s:2:00ff`是两个原bytes，`nil`是一次未命中；string的length必须与hex一致。
匹配成功仍先输出两个闭区间坐标，随后按左括号打开顺序输出捕获。位置捕获`()`
不是普通空字符串捕获；例如 `(a*)` 可产生`s:0:`，`()`则产生数字位置。
hex在Lua内部由byte/sub/算术编码，不使用find/format，也不依赖stdout文本模式。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract real_binary_matches_fifty_four_fixed_capture_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract capture_corpus_keeps_fifty_four_arities_lengths_and_binary_values -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract capture_errors_keep_success_path_timing_and_distinct_categories -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract rollback_restores_closed_values_positions_and_new_slot_count -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract owned_indices_vm_strings_and_legacy_range_projection_keep_their_boundaries -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract captures_do_not_enable_backreferences_or_erase_pattern_work_limits -- --exact
```

54条覆盖普通/嵌套/空字符串/position、NUL/ff/LF、已有closed槽恢复、新增槽截断、
position替换、候选起点reset、多次调用不泄漏、plain/C-prefix、成功路径才报告unfinished、
孤立右括号与坏集合的可达性。32个普通/嵌套/position/mixed捕获能成功，第33个到达
时失败；未到达的第33个不预先拒绝。`')'`自身没有magic而走autoPlain，非法多余close
用`^a)`强制pattern路径，避免把字面搜索误当capture语法验证。

`find_captures`返回拥有的`FindMatch`与`Capture::{Bytes(Range),Position(offset)}`
索引，位置为0-based；这些数字不借用subject，也不使原subject存活。VM成功后才按编号
把range复制为owned ByteString，position加1；本地合同在清payload、GC及另一轮find后
核返回bytes仍在。旧`find`保留Range投影，也必须验证unfinished，不能通过丢掉captures
把错误变成功。回退同时恢复既有槽内容和槽数；已消耗steps不回滚，retry深度限制同时保留的栈帧数。

checker缺省仍为base75；显式选择captures54或all129，`--check`不启动binary：

```powershell
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --check --suite captures
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --check --suite all
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --suite all --candidate-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --official-bin target/oracles/lua-5.1.5/build/Release/lua51.exe --official-archive target/oracles/lua-5.1.5/lua-5.1.5.tar.gz --output target/teaching-v100-captures-2026-10-15/official-contract
```

all归档按suite/role/case分开，汇总有每套fixture/driver hash；base缺省仍沿原目录协议。
同一固定官方source tar/binary身份沿用上文，每次实际重新记录，captures的源依据是
lstrlib.c的start_capture/end_capture/push_onecapture/str_find_aux，不冒充新增C++差分。
pcall false仍只固定稳定标志；本地Type/Limit与原因另验。第33个capture在候选内分类
为Limit，区别于unfinished/孤立close的Type，不要求Lua C运行时具有同一Rust错误枚举。
普通/position捕获已实现，backrefs、`%b/%f`、optional/non-greedy、gmatch/gsub与locale
仍保持D005 partial，不能因capture返回成功宣称完整Lua pattern。
