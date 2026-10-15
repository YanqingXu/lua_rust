# string.format 的有界字节合同

从仓库根执行固定本地合同（Release 在 `--` 前加 `--release`）：

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract real_binary_matches_fifty_seven_fixed_format_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract fixed_format_corpus_has_fifty_seven_raw_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract format_type_errors_keep_kind_and_argument_number -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract missing_values_take_priority_before_format_parsing -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract invalid_format_errors_do_not_become_argument_errors -- --exact
```

`tests/fixtures/string-format/cases.tsv` 的57项 expected 是先手写固定的值，再由真实
Lua 5.1.5 核对；不把 candidate 输出反生成 oracle。`driver.lua` 独立列出57次调用，
按 `arg[1]` 选择一个 case；每次新建一个进程。固定成功值保存在 `value_hex`，完整
stdout 必须为 ASCII `ok|<value_hex>|after`；错误为 `error|after`。每项 exit0、
stderr 空。Rust 测试独立逐 byte 核对完整流，不要求机器安装外部 Lua。

这里的 hex 发生在 Lua 程序内部：formatter 先产生 bytes，再用 byte/sub/整数运算
编码，编码器不调用 `string.format`。Windows 官方 Lua 的 C stdout 文本模式会将
实际 LF 改为 CRLF；提前编码能观察 `%q` 中反斜杠加真实 LF、NUL 与0xff。Python
和 Rust 捕获后的输出完全不改写，不 trim、不有损解码、不做 CRLF normalize。
`%q` 的真实 LF 和反斜杠加字母 n 是不同 bytes；另用 `loadstring` 回读原 payload。

错误 case 用 pcall 的 false 保持跨引擎输出稳定，不伪造两边诊断文字/trace 相同。
三个本地错误测试另查每类错误的 `VmErrorKind::Type`、参数号、错误类别，以及先
检查缺参再解析非法格式的优先级。因此无关异常不能仅凭 pcall false 冒充本地错误
合同通过。未知 case 的 assert 在 pcall 之外，脚本编译错误、未选中 case 均会失败；
after 只在选中 closure 返回或被 pcall 捕获后写出。

先做 schema/driver 身份检查：

```powershell
py -3 book/code/v100-lua51-runtime/tools/check_string_format_contract.py --check
```

它明确打印 candidate/official NOT_RUN，只代表格式检查。实际对照必须同时提供两边
binary 和固定官方 source tar；缺少其中一个就失败。以下命令适用于本仓库 Windows
x86_64 配置与已准备好的官方 build；先 build 当前 candidate，路径随 target-dir/
triple 调整。复制单独快照时，也可提供外部准备好的同一官方归档和 binary：

```powershell
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app
py -3 book/code/v100-lua51-runtime/tools/check_string_format_contract.py --candidate-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --official-bin target/oracles/lua-5.1.5/build/Release/lua51.exe --official-archive target/oracles/lua-5.1.5/lua-5.1.5.tar.gz --output target/teaching-v100-format-2026-10-15/official-contract
```

固定参照是 [Lua 5.1.5 source](https://www.lua.org/ftp/lua-5.1.5.tar.gz)，归档 SHA-256
`2640fc56a795f29d28ef15e13c34a47e223960b0240e8cb0a82d9b0738695333`。源码
`src/lstrlib.c` 的 `scanformat/addquoted/str_format` 决定 flags、两位 width/precision、
缺参先检查、短 `%s` 的 C-NUL 截断与无 precision 且长度>=100的原 bytes fast path；
`src/luaconf.h` 的 `LUA_NUMBER_FMT="%.14g"` 决定数字转 `%s/%q` 以及格式串自身
数字参数的表示。首次对照的本地官方 binary SHA-256 是
`47fd81e426c4f598e9fe2fdf28a0b36df721984ecfd66c8991eeef763693978d`，PE machine
0x8664（x64）；工具每次重查实际版本并记录实际 binary hash，不要求别的平台编出
相同 binary。归档和可执行文件是分别记录的身份，版本字符串不等于可复现构建证明。

每次运行创建独占目录，保存双方各57个 case 的 stdin、实际/预期双流 bytes 与
metadata；汇总另存 archive/源码成员/fixture/driver/checker/binary SHA、版本原流、
平台与 timeout/reaped。`communicate` 同时读取双流并关闭 stdin；超时后 kill，再
communicate/wait，失败同样归档。输入输出有界且无后代，不外推任意进程树清理。
错误/超时/写入失败不报通过，旧目录不覆盖；不声称 fsync/断电原子归档。

本包只收口已有 `s/d/i/f/q/%%` 的列明规则：默认 f precision6、flags/width/precision、
字符串/数字 coercion、NUL/高字节/long fast path 与错误边界。整数验收限共同可表示
的有符号32位范围；浮点只验有限普通值，避开 halfway rounding、极大数、非有限、
locale/CRT 差异，数字 `%.14g` 也只是列明代表值。未添加 x/o/u/e/g 等转换全集，
pattern、dump 仍有缺口；`LUA51-D005` 保持 partial。这不是旧五项语言双 oracle
差分的新结果，不覆盖或重写旧 baseline。
