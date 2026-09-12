# M2 错误来源与名称回溯修复

本包以 `lua_cpp@87c15e6` 为项目 oracle、官方 Lua 5.1.5 为第二 oracle。遵循 `plan.md` §3.1 的仲裁规则：下表列出的新冲突是待决事项，不是已批准 deviation。本包没有修改 debt baseline，也没有迁移 C++ 的回调吞错或 xpcall 异常行为。

## 已修复范围

- `RuntimeError` 新增独立的诊断来源元数据，持有已复制的 chunk id 和行号，不持有额外 GC 引用。算术与 UNM 在出错帧展开前捕获来源；普通 protected call（包含协程内 pcall 与 xpcall 的被保护阶段）和 main 错误出口才物化前缀。直接 coroutine.resume 保留固定 C++ 的原始算术消息。
- 显式 Lua error object 与诊断分离。`error(obj)` 保留身份，`error(str,0)` 保留字节；`error(str,1)`、已有位置的错误与已经物化过的诊断只出现一次来源。移除 CALL 对 `message.starts_with("bad argument")` 的文本猜测，修复 `error('bad argument fake',0)` 被错误加前缀。
- 内部 native result-count 协议新增具名 `NATIVE_ERROR_DIAGNOSTIC=-2`；既有 `-1` 为 `NATIVE_ERROR_OBJECT`。string 的统一诊断入口改为 -2，覆盖 CALL、TAILCALL、直接 `pcall(string.char,256)`、协程及回调内 pcall。native `finish_native_call` 是唯一负返回值消费者；pending request/yield 的处理次序保持原有规则。全源搜索没有此前生产回调占用 -2；其他未知负数继续按既有显式 error object 处理，绝不当正常结果数。此协议是 Rust 内部 ABI，不是外部 Lua C API 的 longjmp 合同。
- `string.dump` 的既有未实现结果继续作为显式错误对象返回，保持稳定 unsupported 文本；没有为迎合新定位逻辑修改旧断言。
- 名称回溯不再把无关 JMP 当成名称丢失屏障；按固定 C++ 检查相应寄存器的 TEST/TESTSET。修复官方 `errors.lua` 的 `insert(prefix,a)` 全局名丢失，同时保留短路表达式的匿名诊断。相邻双 oracle 一致修复包括 CALL 的 table 类型、无名称算术操作的实际类型，以及非函数 TAILCALL 的 Lua 调用错误。

`tests/lua/tables/test_metamethod_arith.lua:141` 是普通 pcall 内两个无元方法的表相加，不经过有争议的回调。修复前其错误消息没有来源；修复后 Debug、Release、固定 C++ 的完整 stdout、stderr、exit 逐字节一致，均退出 0。错误行是：

```text
Expected error: G:\github\lua_rust\tests\lua\tables\test_metamethod_arith.lua:141: attempt to perform arithmetic on upvalue 't1' (a table value)
```

官方消息使用相同行号和错误文本，但按官方 chunk id 规则截断长路径。之前完成的数字 formatter 加上本包来源修复，足以关闭该具体 debt，无需扩大 normalization 或更新失败签名。

## 验证及证据

- 本包当时 `crates/lua_app/tests/error_provenance.rs` Debug 3/3，递归用例另列 ignored；后续迭代调用整改已启用该用例，Debug/Release 各 4/4，见[普通调用整改](2026-09-12-recursive-call-remediation.md)。
- `error-provenance.lua` 固定 C++ 合同 fixture：当前 Rust Debug、Release、固定 C++ 全部通过。覆盖 main pcall、resume、协程内 pcall、xpcall、普通嵌套调用、字符串 native CALL/TAILCALL/direct pcall，以及 native request callback 内的受保护诊断；覆盖 `@file`、`=name`、inline、长文件名、长 snippet。
- `error-object-and-names.lua` 双 oracle 共同合同 fixture：Rust Debug/Release、固定 C++、官方全部通过；包含任意字节错误字符串、table/boolean/number/nil 对象、显式已定位消息、metamethod/gsub 传播对象，以及 10 个名称和类型相邻用例。
- 现有 string 相关 stdlib integration 19/19、string error boundary 3/3，含稳定 string.dump unsupported 断言。
- 本 agent 的独立 Release cargo test 在链接替换 exe 时遇到 Windows 文件占用（拒绝访问），没有伪称该 cargo 命令通过；上面的 Release fixture 结果来自可执行三方差分。最终统一 Release/quality 验证由 root 执行。
- 原始三方矩阵、参数、退出码、stdout/stderr、可执行 SHA-256 在 ignored `target/error-provenance-evidence/before.json`、`verification.json`；`probe.py` / `verify.py` 可重跑。`native-neighbors.json` 补充尚未迁移的 native 参数错误观察。

## 新发现的 oracle 冲突与保留范围

矩阵 chunk 为 `=origin`；下面的行号区分外部调用与 callback 内实际出错行。所有探针均以 pcall/resume 捕获结果，三引擎矩阵进程均退出 0。

| 路径 | 固定 C++ | 官方 5.1.5 | 本包处理 |
| --- | --- | --- | --- |
| 直接 coroutine.resume 算术/UNM | 无来源前缀 | 实际 Lua 行前缀 | 保持原已采用固定 C++ 合同 |
| pcall 中普通 Lua 嵌套调用 | 实际出错行 `origin:3` | 同左 | 已修复 |
| pcall 中 `__add` callback 算术错误 | 外部操作行 `origin:4` | callback 出错行 `origin:3` | 新冲突，未迁移；Rust 该 helper 仍返回原始消息 |
| pcall 中 gsub/sort callback 算术错误 | native 调用行 `origin:1` | callback 出错行 `origin:3` | 新冲突，未迁移；Rust 该 helper 仍返回原始消息 |
| xpcall 中 `__add`/gsub callback 算术错误 | `error in error handling` | handler 正常收到错误 | 不复制 C++ 异常，待仲裁 |
| `tostring` 的 `__tostring` 抛错 | 调用成功，返回带位置的错误字符串 | 调用失败，传播错误 | 保留 Rust 传播错误，不复制吞错 |
| 长文件名 / snippet | 完整文件名 / 60 字节 snippet | 文件名 / snippet 更短 | 使用既有固定 C++ chunk renderer |
| string.char 参数错误文本 | `string.char` / `value out of range` | `char` / `invalid value` | 保留此前有证据的固定 C++ 文本 |

既有 `runtime_error_at` 调用点（例如 index、CALL、FORPREP）仍在原位置立即形成已定位错误；本包没有假称所有 VM 错误已完成统一策略迁移。native string 之外的多个入口仍返回 -1，例如 math、IO、rawset；应分别区分诊断与显式对象再迁移，不能恢复字符串启发式。移除启发式后，某些原本靠它得到前缀的非 string `bad argument` 消息不再获得猜测性前缀；此限制已交给整合 gate 检查，不能靠扩大 debt baseline 掩盖。

## 历史发现及后续修复：Debug 宿主递归栈溢出

修复 `insert` 后，完整官方 `errors.lua` 越过首个 checkmessage 阻塞并打印 `+`，在后面的递归测试发生宿主栈溢出。最小复现见 `tests/characterization/recursive-error-boundary.lua`：非尾调用函数递归，外层 pcall 应拿到 Lua stack overflow 并继续执行。

| 实现 | 最小复现结果 |
| --- | --- |
| Rust Windows Debug | exit `3221225725` (`0xC00000FD`)，`thread 'main' ... has overflowed its stack`，不能继续 |
| 当时 Rust Windows Release | exit 0，受控 Lua 错误后打印继续执行标记 |
| 固定 C++ / 官方 5.1.5 | 均 exit 0，受控错误后继续 |

当时 `execute_counted_proto` 的 Lua `MAX_CALLS=512` 检查依赖 Rust 递归执行链，Debug 中宿主栈先于该阈值耗尽。随后已将普通 Lua 调用改为迭代帧切换，并以活动帧数独立检查预算；原 ignored 用例已启用，Debug/Release 和 1/2/8 MiB 宿主线程栈的独立进程回归通过。详见[普通调用整改与验证](2026-09-12-recursive-call-remediation.md)。这不表示官方 errors 文件全部通过，错误原始字节等后续断言仍独立追踪。
