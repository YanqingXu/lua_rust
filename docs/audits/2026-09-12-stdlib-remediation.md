# A03/A04：标准库进程错误边界修复

基于审计提交 `503bf99`，在 Windows x86_64 本地工作区验证；固定 C++ oracle 为 `87c15e69ceb94eb74e28226ccbefb7e196635711`。本文件记录实际修复和仍存在的差异，不是新增兼容偏差的批准记录。

## 修复范围

- `rawset` 在进入底层表写入前拒绝 NaN/nil 键、缺少参数及非表参数，先压入字符串错误对象，再通过 VM 的错误返回合同返回。NaN 不再触发穿过 `extern "C"` 的 Rust panic。
- `os.time` 的六个数值字段按固定 C++ 的 `checkedLuaInteger` 合同处理：必须有限、先向零截断小数、再检查 i32 上下界。日期字段按 `sec/min/hour/day/month/year` 顺序检查；必需字段、可选字段及非数值字段采用固定 C++ 的规则。
- 时间换算先提升为 i64，再归一化月份和执行受检秒数运算；合法的 i32 端点也不再导致年月偏移或日期乘法回绕。转换失败返回 nil；无效数值字段返回 Lua 字符串错误。64 种六字段上下界组合测试覆盖了接受范围的极端算术。
- `next` 接入表层 `try_next`，无效游标产生字符串错误 `invalid key to 'next'`；GC 访问失败单独报告，不能伪装为正常迭代结束。
- 环境/元表/loader fixture 在创建环境表时注入 `tostring`，保留 `writes.count == 6` 和原有退出码要求，后半部分断言现在实际执行。

## 验证

| 验证 | 结果 |
| --- | --- |
| `cargo test -p lua_app --test stdlib_error_boundary` | 3/3 通过 |
| `cargo test -p lua_app --test stdlib_error_boundary --release` | 3/3 通过 |
| `cargo test -p lua_stdlib --lib os::byte_string_tests` | 3/3 通过 |
| 固定 C++ 运行 `tests/characterization/rawset-error-boundary.lua` | 通过 |
| 固定 C++ 运行 `tests/characterization/os-time-error-boundary.lua` | 通过 |
| Rust Debug/Release、固定 C++、官方 Lua 5.1.5 运行 `tests/lua/integration/test_env_metatable_loader.lua` | 四次均退出 0，执行到末尾 passed，writeCount 为 6 |

子进程测试涵盖 `pcall`、协程直接失败、协程内部捕获后继续 yield/resume，以及同一进程执行下一条 `-e` 脚本。时间矩阵涵盖每个字段的正负超大有限值、NaN、正负无穷、i32 越界、缺失字段、错误类型和小数截断。单元测试还覆盖截断后的精确 i32 接受边界以及跨年/月/闰年归一化。

## 双 oracle 的明确差异与后续工作

| 输入 | Rust 修复后 | 固定 C++ | 官方 Lua 5.1.5 |
| --- | --- | --- | --- |
| `pcall(rawset, {}, math.sqrt(-1), 1)` | false + `rawset: table index is NaN` | false + 位置前缀及同一消息 | false + `table index is NaN` |
| `pcall(os.time, {year=1e300,month=1,day=1})` | false + `date table field has no valid integer representation` | false + 位置前缀及同一消息 | true + nil |

本次锁定受控失败、错误对象类型和 C++ 消息内容。Rust 标准库 callback 错误尚未统一添加 C++ 的调用位置前缀，完整错误文本 parity 仍需在诊断合同工作中收口。没有把上述官方差异写成已批准 deviation，也没有宣称三种实现的所有错误返回相同。

此外，Rust 原有 `os.time` 使用纯 UTC 日期换算；固定 C++ 使用本地时区的 `mktime`，日期范围还受平台 C 运行库限制（例如 Windows 对极大但可表示的 i32 年份返回 nil）。此次输入检查与算术安全修复没有完成时区、DST 或平台时间范围兼容。该域差异应单独设计和验证，不能用本次进程安全测试的通过代替 OS 库整体兼容验收。
