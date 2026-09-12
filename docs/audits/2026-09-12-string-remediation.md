# A08：字符串参数与错误对象修复

固定 C++ 合同来源：`87c15e69ceb94eb74e28226ccbefb7e196635711` 的 `src/lib/stringlib.cpp`、`src/common/number_conversion.hpp`、`src/runtime/resource_policy.hpp`；同时运行官方 Lua 5.1.5 对照共同语义。本文是修复和差异记录，不构成新 deviation 批准。

## 已完成

`string/args.rs` 使用 `Result` 区分缺省参数、错误类型、不能表示为整数的数值。所有涉及整数位置、字符、次数和整数格式化的入口在浮点转换前检查有限性及截断后的 i32 范围；数值字符串支持首尾空白、十六进制和小数。callback 通过共同的错误出口压入字符串，再返回负结果数，不再把原始参数错误地当作 error object。

修复覆盖 `byte/char/find/match/sub/gsub/rep/len/upper/lower/reverse/gmatch/format` 的相关参数。`find(..., true)` 跳过模式语法验证；`gsub` 在没有捕获时仅允许 `%1` 引用全匹配，`%2` 等非法索引返回错误。`format` 的 `%s/%q` 拒绝表和布尔值，整数转换遵循 i32 合同，无符号转换按照 u32 解释负整数。

`rep` 检查乘法、分配结果和输出大小；`format` 在解析宽度/精度时检查整数溢出及上限，并检查累计输出；普通字符串 replacement 的 `gsub` 也检查累计输出。当前上限为固定 C++ 默认的 64 MiB；Rust 尚未提供统一的、可由宿主配置的 native 资源策略。本次限制不是完整的内存配额或可恢复 OOM 合同。

## 验证入口

- `cargo test -p lua_app --test string_error_boundary` 与同命令 `--release`：真实子进程验证参数/error object、协程失败与捕获恢复、下一条脚本继续执行、超大格式参数和非法模式受控失败。
- `cargo test -p lua_stdlib --lib string::args`：字符串数值解析、i32 精确边界、NaN/Inf、上溢及下溢。
- `cargo test -p lua_stdlib --test stdlib_integration_tests string_`：现有匹配、替换、嵌套回调、格式化、任意字节等 19 项回归。
- `tests/characterization/string-argument-boundary.lua`：固定 C++ 的完整定向参数矩阵。
- `tests/characterization/string-common-error-contract.lua`：Rust、固定 C++、官方 Lua 共同合同，包括审计中的四例、返回数量与 replacement 回调错误对象身份。

## 仍有区别，不能当作整体字符串库兼容完成

| 场景 | 本次 Rust 合同 | 固定 C++ | 官方 Lua 5.1.5 |
| --- | --- | --- | --- |
| `byte/find/sub/gsub` 的显式 nil 可选数值参数 | 类型错误，缺省才使用默认值 | 相同 | nil 可作为缺省 |
| `gsub('a','a','%')` | 空字符串 | 空字符串 | 一个 NUL 字节 |
| `find('abc','[')`、末尾 `%`、不完整 `%b/%f` | 保持已有的字符串模式错误 | 多数返回 nil | 字符串模式错误 |
| 错误文本位置 | callback 错误没有统一位置前缀 | 通常含调用位置 | 按各 API 错误规则定位，消息也有差别 |

本次共同矩阵锁定两个 oracle 一致的结果/错误类型/返回数量；固定 C++ 矩阵锁定其参数消息后缀。没有将官方差异写成已批准豁免。超大格式宽度/精度采用提前拒绝，尚未对所有特殊格式与两种 C 运行库的数值解析边缘行为宣称逐字节相等。模式引擎的完整兼容、最坏回溯工作量与递归深度预算也不属于本次已关闭项。
