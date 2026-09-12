# 共享 Lua number formatter

新增 `lua_core::number_format`，Lua 可观察的缺省数字转换统一采用 14 位有效数字，与固定 C++ `to_chars(general,14)`、官方 Lua 5.1.5 `LUA_NUMBER_FMT "%.14g"` 对齐。未更改 Value equality、编译器常量位型、bytecode JSON 或 trace 的数字序列化。

实现先使用 Rust 的科学计数格式将二进制值舍入到请求的有效位，再根据**舍入后的指数**构造 general 格式。这样既修复 `10/3` 的输出，也修复两处显式 `string.format('%.14g')` 边界：

| 数值 | 修复前的显式 %.14g | 修复后及双 oracle |
| --- | --- | --- |
| 99999999999999.5 | 100000000000000 | 1e+14 |
| 0.0000999999999999995 | 1e-04 | 0.0001 |

接入点涵盖 print、tostring、VM concat、普通/回调/table replacement 的 gsub、字符串数字参数转换、table.concat、IO write、package 错误值文本与 REPL 值展示。显式 `%g/%G` 复用同一舍入逻辑，保留用户精度、alternate、符号、宽度与对齐；`%f/%e/%E` 的非有限数值也使用同一平台 token。

负零保留符号。NaN 不通过绝对值运算丢失原始位型；Windows 按 MSVC/UCRT 的 canonical indefinite、quiet 和 signalling 分类生成 `nan(ind)`、`nan`、`nan(snan)`，再保留符号。非 Windows 使用 `nan`/`-nan`。生产实现完全使用 Rust，未新增 libc 或其它依赖。

## 验证

- `cargo test -p lua_core --lib number_format`：3 项通过，覆盖 14 位阈值、最小 subnormal、最大有限数、正负零/无穷/NaN payload、alternate 与显式精度。
- `cargo test -p lua_app --test number_format` 和 `--release`：各 2 项通过，覆盖 16 值 × print/tostring/concat/%s/%.14g/IO 六渠道，另验 table.concat、gsub 三类 replacement、数字字符串参数及格式 flags。
- `cargo test -p lua_stdlib --test stdlib_integration_tests string_`：19 项既有集成测试通过。
- `cargo clippy -p lua_core -p lua_stdlib -p lua_app --all-targets -- -D warnings`：通过。
- 固定种子的 1,076 个有限数值（随机 f64 位型及指数阈值邻域）× 16 种 `%g/%G` 精度/flags，另含隐式转换：8 批 × 三引擎均正常退出，Rust 对固定 C++、官方 Lua 均零差异。
- 长期 fixture `tests/characterization/number-format-matrix.lua` 的三引擎 raw stdout SHA-256 均为 `75c3d50f42e39316da8476677f8122f056378f702770d47397793e97b1e824a5`。
- 独立 MSVC 19.51 C++ 小程序直接比较 10 种零/NaN 位型的 `std::to_chars` 与 `printf("%.14g")`，两者完全一致；对应分类已转成 Rust 单元测试。
- 同一 C++ 位型探针在 WSL Ubuntu 24.04 下以 g++ 编译并执行，10 项 `to_chars`/printf 相同，NaN 均为 `nan` 或 `-nan`，确认本次 Linux 分类与 Rust 非 Windows 分支一致。此项是 C++ 平台 token 验证，不是 Linux workspace 或随机差分全量验证。
- `cargo test -p lua_app --test observable_io repl`：2 项 REPL 针对回归通过。

可复跑的随机差分脚本、每批 Lua 源码、raw 输出、executable hashes、NaN C++ 小程序与结果位于 ignored `target/number-format-evidence/`，主摘要为 `differential.json`。上述进程差分记录来自 Windows x86_64；不据此宣称所有平台的 C 运行库行为均已验证。

本工作包没有修改 non-official debt 签名、userdata print 选择或错误 source-prefix 合同。`tables.metamethod-arith` 的数字部分已对齐，但错误来源部分仍须独立收口，不能只因数字文本修复而直接移除整条 debt。
