# 普通 Lua 调用的宿主栈边界整改

状态：本轮普通调用边界整改已验证，后续 M2/M3 继续推进。基于 `503bf99` 上已有审计整改工作区；本文件不将前一轮
完整门或官方执行快照作为本轮修改后的通过证据。

## 问题与范围

修复前，400 层有限非尾递归在 Windows Debug 中触发 `0xC00000FD`，而同一源码在
Rust Release、固定 C++ 和官方 Lua 中都返回 400。更复杂的分配密集递归还暴露
旧 Release 超时，该运行记录独立保留。静态检查确认原 `nccalls` 随 Rust 执行器返回
而恢复，不能限制跨 Runtime 暂停持续增长的 Lua 调用栈。无限递归需要受控失败，
有限合法递归需要保留正确结果。

本轮处理普通 Lua 调用的调度、返回后处理与 retained-frame 预算。它不声明任意
宿主线程栈大小、外部 Rust/C 回调自身的递归、OOM 或所有运行时入口已经安全。

## 实现

- 普通 CALL、`__call`、Lua TFORLOOP 和 synthetic root 的 Lua TAILCALL 在同一个
  `execute_proto_inner` 循环中切换活动 Proto、PC 和 CallInfo；每层 Lua 调用不再
  保留一层 Rust 执行器调用。普通 tail call 继续复用当前 Lua 帧。
- CallInfo 保存返回后的操作，Lua iterator 的控制寄存器更新和 nil 时跳过指令
  由真正返回的路径执行一次；状态可跨 coroutine、GC 和 debug-hook 暂停保留。
- 每次入栈根据活动 Lua 帧检查既有 512 上限，独立于一次同步执行的 trace 计数。
  暂停后重新进入执行器不会恢复这份预算；native 帧不混作 Lua 帧计数。
- 普通入口和 hook 指令恢复入口共享执行作用域。主状态错误关闭并弹出本次入口
  之上的 Lua 帧；protected/helper/finalizer 的调用者边界仍由 Runtime 所有者处理。
  失败协程保留可供 traceback 检查的帧与 open Upvalue。

公开宿主同步 helper 仍使用有边界的执行器入口；生产 opcode 不再通过该 helper
执行 Lua。GC 扫描继续使用原有 CallInfo/Value 根，新增 continuation 只存标量状态。
512 是本轮保留的 Rust 调用限制，不代表已经对齐 C++/官方的全部深度和资源预算合同；
后续应在资源预算工作包中分别核实 Lua 帧上限、native 重入上限及错误处理余量。

## 验收与证据

定向测试为 `crates/lua_app/tests/error_provenance.rs`、
`crates/lua_stdlib/tests/recursive_call_boundary.rs`，共用
`tests/characterization/recursive-error-boundary.lua`。
多栈测试在每个独立子进程内启动指定栈大小的宿主线程，并在线程内部创建、初始化、
执行和关闭 Runtime；进程异常退出与超时均为失败。

冻结生产代码后的定向结果：

| 检查 | 结果 | 证据 |
| --- | --- | --- |
| CLI error_provenance | Debug/Release 各 4/4；原递归 ignored 已移除 | `target/recursive-call-evidence/test-results.json` |
| 多宿主线程栈 | Debug/Release 的 target 各 2/2；driver 内 1/2/8 MiB 各自隔离子进程全部通过，关闭后对象和托管字节均为零 | 同上及 `artifacts.json` |
| Lua fixture 双 oracle | 固定 C++/官方均退出 0，全部断言通过 | `target/recursive-call-evidence/oracle-evidence.json` |
| 隐式 EOF iterator | Nil 结果填充、控制值与跳转 continuation 单测通过 | `execute::proto_handle_tests::empty_iterator_proto_returns_nil_and_completes_its_caller` |
| 迭代 Lua 帧/GC/协程/尾调用 iterator | 默认 Miri 1/1，227.84 秒；未设置 MIRIFLAGS 或关闭别名检查 | `target/recursive-call-evidence/miri-iterative-frames.log` |
| 最终完整 M0 | `passed=true`、`fullGatePassed=true`，零硬失败，123,673 ms；92 执行 / 83 match / 9 debt | `target/recursive-call-evidence/m0-verified.json` |
| 最终完整 M1 | `foundationPassed=true`，零硬失败，117,162 ms；Debug/Release 各 1,009 普通测试，无 ignored，另有 10 doctests | `target/recursive-call-evidence/m1-verified.json`、`test-list.log` |

fixture SHA-256 为 `9a7975db2d8340e6bfdc2eb1db9277b616d2a4b1124bd09d320665471c9f1bc8`。
它覆盖 400 层有限非尾递归、多轮分配递归超限恢复、捕获局部值的错误展开、10,000
次尾调用、`__call`、call/return hook、存活帧 GC、协程暂停恢复，以及 iterator
经 pcall/GC/hook 暂停并尾调用返回后控制值递增与 nil 退出。

额外探针发现 iterator 内直接 yield 的 oracle 冲突：固定 C++ 首次 resume 返回成功
及空结果后结束协程；官方拒绝跨 metamethod/C-call boundary yield。本轮没有将其中
任何一种写成新批准合同，最小探针在 `target/recursive-call-evidence/iterator-probe.lua`。

首次完整 M1 的失败产物 `m1-complete.json` 保留：源码静态检查禁止 `.as_ref()` 文本，
本轮用于普通 Result 借用的同名方法触发该规则。随后改用等价模式匹配，未调整 guard，
再执行上述完整门。定向 Miri/进程运行早于这处 Result 借用语法调整；最终完整门覆盖调整后源码。

最终源码与 fixtures/tools/CI 共 360 个文件的原始字节已单独归档，含未提交改动：
`target/recursive-call-evidence/source-final.zip`，清单 SHA-256 为
`dc58a49beff50b57a21c953645596ff8a232b29d4ba7043b38115cfd8a333edf`。
门禁从未变化的工作区构建；此归档用于保留输入身份，不声称门禁是从 ZIP 重新构建。
M1 的 `m1Complete=false` 仍保留，公开 allocator/自动 GC 上界等工作没有由本修复代替。

原始复现保留在 `target/recursive-call-evidence/`，特别是
`finite-400-before.json`；旧失败产物不会被修复后的运行覆盖。

## 后续顺序

普通调用边界验收后，接着处理官方 `db.lua` 的 line hook 返回顺序。只读对照已经
确认旧 Rust 使用线程级 last-line/PC，callee 覆盖 caller 历史；固定 C++ 和官方按
活动调用帧保留历史。六个小探针的证据在
`target/audit-fixes-2026-09-12/debug-hook-diagnosis.json`，使用先前冻结的旧二进制。
修复需要覆盖同一行返回、递归、回跳、在 callee 中安装 hook、count/line 顺序与
tail call 帧复用；本轮没有在调度验收期间混入该项生产修改。
