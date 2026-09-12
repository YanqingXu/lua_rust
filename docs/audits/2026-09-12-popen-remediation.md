# 2026-09-12 Windows popen 可用性整改

状态：实现和定向验证完成，与嵌套 codegen 修复一同纳入本次提交。基线为
`33aba8e`，固定 C++ oracle 保持 `87c15e69ceb94eb74e28226ccbefb7e196635711`，
第二 oracle 为官方 Lua 5.1.5。远程 CI 必须按本次提交独立核对。

## 行为与修复范围

固定 C++ 在 Windows 仅对精确命令 `ls` 执行 `system("where ls >NUL 2>NUL")`。
命令不可用时，在创建管道前抛出包含调用位置的 `io.popen: command not available`。
旧 Rust 直接创建 pipe，随后 close 失败，导致原版 files.lua 无法沿上游不可用分支继续。

Rust 已在模式校验之后、pipe 创建之前补齐该检查。Windows helper 处理 COMSPEC，
按已验证的 UCRT 行为先尝试原路径，basename 不先查 PATH，仅在启动失败属于
NotFound/PermissionDenied 时回退 cmd.exe；启动成功但返回非零不回退。
探针执行固定字符串，不拼接 Lua 参数。返回 NativeDiagnostic，让 VM 补齐 Lua 调用位置。
其他命令、普通 pipe 生命周期和 close 返回合同未改。

本机 Windows SDK 10.0.26100.0 的 UCRT system.cpp/spawnv.cpp 是对应源码证据。
中间版本增加 CREATE_NO_WINDOW 会改变自定义 COMSPEC 程序的原生 stderr 字节；
两个直接原生进程确认是该标志导致。最终生产 helper 已删除该标志，两项独立探针
与固定 C++ 的 stdout、stderr、退出码完全一致，没有用输出抑制隐藏差异。

## 验证与输入身份

- 新增 Windows 进程回归测试 Debug/Release 各 8/8，无 ignored。覆盖不可用/可用的
  read/write、尾随空格、模式校验顺序、COMSPEC 路径与回退、空值和缺失值。
- 每个最终二进制另完成九项原始回放，无超时；按报告分别校验共有合同和既有差异。
  独立复核另外三项最终探针正常退出：shadow where 与绝对 COMSPEC 的两项完全
  匹配固定 C++；空 COMSPEC 的受控错误单独记录。
- 测试使用子进程私有 PATH、COMSPEC 和目录。输入握手保证 Lua 执行前已进入
  Windows Job Object；截止时间包括输出管道回收，超时终止整个进程树。
- 最终完整 M0 通过：75,542 ms，0 hard failure，83 match / 9 debt，无 smoke 或跳过。
  fmt 与 lua_app/lua_stdlib all-targets Clippy `-D warnings` 通过。
- 嵌套 codegen 的完整 M1 基础门、1,080 项普通测试清单及默认 Miri 两项通过，
  证据属于前一个源码快照，见其独立整改报告。本轮按用户快速收敛要求不再追加
  完整 M1/Miri；不将旧结果描述为最终 popen 源码的全量验证。

Cargo/配置/源码/tests/tools/CI 共 377 个最终输入的清单 SHA-256：
`ed0cc750243c22ccbd089004aebffe32908cfa789afffa2da20836f0fb4ce792`。
从该工作区构建，ZIP 只固定输入身份，不声称从 ZIP 重建。

| 最终 app | SHA-256 |
| --- | --- |
| Debug | `a2b149894295c125696d5763df8358b34f91b060fb788b7f146269f5e215c8b9` |
| Release | `e40fb59e53345083d0505e0c2a3a83bb0c8417afdf0f223da5bc40037f1ca797` |

证据根目录：`target/files-popen-evidence/`。关键入口为 `source-final.json`、
`m0-final.json`、`current-work/report.md`、`independent-boundaries/final-review.md`。

## 原版 files.lua 最终复验

最终 Debug/Release 在继承 PATH 与仅子进程提供 ls.cmd 的可用 PATH 下分别执行，
共四组 × 三引擎、12 个 Lua 进程，无超时。Rust/C++ 八次全部正常退出、含原版
完成标记且 stderr 为空；继承 PATH 保留上游 popen 不可用分支，可用 PATH 无此跳过。
stock 四次均在第 59 行 os.rename 的 Permission denied 失败，没有完成标记。
四组 runner 均准确返回 1，不将这组结果称为三个引擎全部通过，也未加入严格子集。
未修改上游源码，未把含真实时间的 stdout 称为逐字节一致。

四份 `official-final-{debug,release,available-debug,available-release}.json` 及同名前缀
`.validation.json` 保存最终二进制身份、原始输出哈希、完成标记和子进程 PATH。

## 已知边界与下一步

空 COMSPEC 使本机固定 C++ abort，Rust 返回可捕获的 Lua 错误并可继续 GC/执行；
这是显式记录的未对齐项，不能计作 parity 通过。无效 COMSPEC 的后续通用 pipe 行为、
命令/模式内 NUL 的 C 字符串差异、旧模式错误缺少位置和普通 shell 本地化差异未在
本次扩大处理。NotFound/PermissionDenied 对应本轮验证场景，不声称穷尽 UCRT errno。

固定时间戳 1577836800、子进程 TZ=UTC0/CST-8 的六进程探测另确认：Rust 把本地
日期转换当作 UTC；CST-8 下两个 oracle 的本地小时为 8，Rust 为 0，反向 os.time
也相差 28,800 秒。原文件内部自洽的断言不能证明该合同正确。证据在
`target/os-time-boundary-evidence/from-files-probe/observations.json`。
下一阶段先修本地时间/UTC 与 DST 合同，再补受限大负载；本次没有修改 os.rs。
M1/M2 整体及 M3 dump/binary chunk/C ABI 仍未完成。
