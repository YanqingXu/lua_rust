# string.find 的有界官方字节合同

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
Unsupported/Limit 由另外两个测试验证：`?`/`-`、captures/backrefs、`%b/%f` 到达时
显式拒绝；step/retry-depth 耗尽返回 Limit，不冒充 nil，Session 后续仍可用。
这些不是与官方正常支持行为相等的75条之一；plain 路径不使用 pattern budget，
也不宣称整个VM/进程有统一资源限制。gmatch/gsub、captures、locale等继续 D005 partial。

每次运行创建唯一目录，保存双方各75项实际/预期双流、stdin、metadata，汇总另存
fixture/driver/checker/source成员/binary SHA与版本原流。错误/超时也归档，拒绝覆盖；
写入失败不报通过，不宣称 fsync/断电原子性。新证据不覆盖旧 format/REPL 合同，
也不冒充旧五项语言双 oracle 基线的新结果。
