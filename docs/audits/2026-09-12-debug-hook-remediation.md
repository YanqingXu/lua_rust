# 调试行事件与协程 traceback 整改

本工作包接续已提交的 `4024f9e`，固定 C++ oracle 为
`87c15e69ceb94eb74e28226ccbefb7e196635711`，第二 oracle 为官方 Lua 5.1.5。
不修改 vendored `db.lua`、来源清单或 oracle pin。

## 根因与修复

旧 VM 在线程上保存上一条 line/PC，callee 覆盖 caller 的历史。官方 `db.lua`
第一个测试期望 `{2,4,7}`，返回调用者同一行后却多收到 line=19。
逐帧历史现在保存在 `CallInfo`，同一行向前执行不重复触发，PC 回跳仍触发；
递归调用同一个 Proto 各自拥有历史，尾调用复用帧时清空历史。
安装或替换 hook 为所有活动 Lua 帧记录当前位置，hook 自身执行期间保留防重入状态。
count callback 完成后仍为同一条指令选择 line 事件。

历史字段只含行号与 PC，不再保存一次性跳过的 Proto。GC inventory 随删除的
`DEBUG_PROTO` 从 24 类降为 23 类，其余根合同保留。活动帧的 `ACTIVE_PROTO`
及 Runtime callback continuation 继续负责真实 Proto 生命周期。
新增实际回收检查确认没有 Function 的顶层帧仍保留 Proto，清除该帧引用后才回收。

行事件修复后，原版 `db.lua` 继续执行至第 472 行的失败协程 traceback。
旧渲染器人为重复追加首个 Lua 帧；普通调用的迭代调度已经保留所有真实帧，
这段补偿多报了一层。现逐个输出真实帧，保留 native yield/error 与 tail 标记。

独立复核又确认了 debug 访问边界的三个问题：

- 普通 `debug.getinfo(1, "n")` 已能在默认 Miri 下触发 UB：入口提前取得的
  `&mut GarbageCollector` 被随后的 LuaState 动态 GC 读取使之失效，随后又用于
  结果发布。`getinfo/getlocal/setlocal` 现在先完成所有帧、名字与值的读取，
  只在结果发布时借用可变 GC；name-only helper 返回独立数据，不跨读取保留可变 GC。
- 显式 self 线程复用当前 LuaState 借用，跨线程仍经 Runtime 的受限访问。
  self 的 level=1 排除当前 debug C 帧，外部暂停协程的 level=1 则包含其活动 Lua 帧。
- `debug.getinfo(co, 1, "n")` 原先将表发布到被检查协程的栈，调用者收到错误的值。
  现在先收集目标信息，再统一发布到调用者栈。两个 oracle 对这两个帧/结果合同一致。

## 回归与验证

`tests/characterization/debug-line-hooks.lua` 的 13 个场景覆盖同一行与下一行返回、
同 Proto 递归、单行回跳、callee 中安装、hook 内 Lua 调用、尾调用帧复用、
count/line 精确顺序、暂停协程外部安装/替换/清除、显式 self 操作和 hook 内替换。
固定 C++ 和官方均通过；旧 Rust 二进制在第一个场景失败，证明回归能捕获原问题。

`debug-coroutine-traceback.lua` 检查普通与递归错误帧数量、逐次 yield 后报错、
GC 后重复读取、三个 tail 标记和正常结束协程的空 traceback。
两个参考实现均通过；修复前 Rust 在普通错误的单帧检查失败。
两个 CLI 测试都以独立进程运行，并将异常退出、超时、stderr 或意外输出作为失败。

定向来源与产物：

- `target/audit-fixes-2026-09-12/debug-line-hooks-oracles.json`：行事件双 oracle 与旧 Rust 负对照。
- `target/debug-hook-evidence/traceback-probes/`：原始最小探针与派生诊断，派生文件不是官方通过证据。
- `target/debug-hook-evidence/official-db-first.json`：只有行事件修复时的原版失败记录，保留第三个 `+` 后的断言失败。
- `target/debug-hook-evidence/official-db-debug.json`、`official-db-release.json`：移除重复帧后原版 `db.lua` 两构建均完整通过，三个引擎均到达 runner 完成标记，stdout 原始字节一致、stderr 为空。
- `target/debug-hook-evidence/review/`：独立复核探针与参考实现分歧。
- `target/debug-hook-evidence/miri-debug-access-before.log`：默认 Miri 在首个普通
  `getinfo(1, "n")` 上的 Stacked Borrows 红基线，源码身份见
  `miri-debug-access-before-source.json`。
- `crates/lua_stdlib/tests/debug_access_boundary.rs`：普通、自身与暂停协程的帧访问、
  name-only 结果、局部值读写、GC 后恢复及关闭检查。
- `runtime_gc_boundary::debug_hook_lifecycle_preserves_scoped_state_access`：hook 内
  Lua 调用与 GC、self set/get/clear、外部更换暂停协程 hook、恢复和关闭零残留。

冻结后的默认 Miri 两项均通过：`debug_api_reads_precede_gc_publication` 用时
163.54 秒，`debug_hook_lifecycle_preserves_scoped_state_access` 用时 192.75 秒。
命令为 `cargo +nightly-2026-07-29 miri test -p lua_stdlib --test debug_access_boundary --test runtime_gc_boundary debug_`，
未设置 `MIRIFLAGS`，日志为 `target/debug-hook-evidence/miri-debug-verified.log`。
两项都执行到 Runtime close 和对象、协程、托管字节零残留检查。
最终完整门与官方复验结果如下；这些结果也不表示全部 debug API 无 UB。

| 检查 | 冻结源码结果 | 产物（`target/debug-hook-evidence/`） |
| --- | --- | --- |
| 完整 M0 | `fullGatePassed=true`，零硬失败，191,163 ms；92 执行、83 match、9 debt | `m0-verified.json` |
| 完整 M1 / quality | `foundationPassed=true`，零硬失败，209,515 ms；fmt、Clippy、Debug/Release、doctest、rustdoc、audit 全部通过；`m1Complete=false` | `m1-verified.json` |
| 普通测试清单 | 1,013 项，无 ignored；完整 quality 在 Debug/Release 执行全部 targets | `test-list.log` |
| 最终 Debug 官方严格子集 | 9 文件 × 3 引擎全部完成，含 `db.lua` | `official-required-debug-final.json` |
| 最终 Release 官方 db | 3 引擎全部完成，stdout 原始字节一致、stderr 为空 | `official-db-release-final.json` |

最终 Debug binary SHA-256 为
`c54ba3fd90d36d293043d4f7fbef04b6cfcefc686f1995ca4ad1da2366cb9713`，
Release 为 `1756d08436ec267a3cc09da60eec75d422fa866e36759168943b4f2753c6c85b`。
早先 `official-db-debug.json` 与 `official-db-release.json` 在追加 debug 访问修复前执行；
上表两个 final 产物覆盖最终生产代码，旧结果保留其各自 binary 身份。
CI 的严格官方源码子集新增 `db.lua`；失败仍返回非零。
默认 Miri CI 同时接入新的 debug 访问 target，现有 lifecycle target 自动包含新增 hook 场景。

第一次 hook lifecycle Miri 已执行所有 Lua 断言，随后因测试错误要求“同时借用状态
峰值必须为 1”失败。Runtime 明确允许当前执行状态与被检查协程的受限借用同时存在，
本场景峰值 2 合法。移除该错误断言，保留行为、activation 清理和最终零残留检查；
没有改动 Runtime 借用规则。失败日志 `miri-hook-lifecycle.log` 保留，不能作为通过证据。

本轮 Cargo、配置、源码、fixtures、tools 与 CI 共 365 个输入文件已冻结归档：
`target/debug-hook-evidence/source-final.zip`，清单 SHA-256
`f87ec0ad8591e6a72dea86da80af6d506cc65f27164635eb5a9da901cf5e8099`。
测试从同一未变化的工作区构建；归档保存原始输入，不声称从 ZIP 重新构建。
该工作包验收时尚未提交，现随联合批次提交；先前 `4024f9e` 的远程 CI 通过不覆盖这些新增变化。

## 边界与后续

在 hook 内先清除再重装 callback 的行为仍有双 oracle 分歧：固定 C++ 会在旧
callback 尚未结束时进入 replacement，官方 Lua 保持防重入直至旧 callback 结束。
这不是本轮新引入的行为；当前保留既有 C++ 口径，未把它登记为新批准 deviation。

官方 19 个入口的旧完整快照仍为 11 通过、5 失败、3 超时。本轮先单独关闭
`db.lua`，不把定向重跑推算成另一份完整 19 项结果。后续依次处理 `errors.lua`
原始错误字节、`main.lua` 交互续读、`files.lua` 平台行为和大负载；binary chunk
仍属于 M3 前置工作。官方文件通过也不表示所有 debug API 或完整 `all.lua` 已覆盖。
