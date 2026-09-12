# REPL 续行与错误阶段整改

本工作包接续 [诊断字节整改](2026-09-12-diagnostic-bytes-remediation.md)，
固定 C++ oracle 仍为 `87c15e69ceb94eb74e28226ccbefb7e196635711`。
生产改动仅在 `lua_app` 的交互路径，不改变共享 Parser 的批量诊断。

## 原因和修复

原版 `main.lua` 输入裸名称 `a`，随后换行 `= 10` 时，Rust 将第一行判为终止的
语法错误，第二行转成新的 `return 10`，因此变量始终为 nil。固定 C++ 的 Quiet
交互路径在首行解析失败后，对整个输入做裸名称检查，允许继续等待赋值。

`is_pending_assignment_name` 现在实现该规则：按 C locale 的六种空白裁掉首尾，
名称首字节为 ASCII 字母或下划线，后续为 ASCII 字母、数字或下划线。条件只用于
Quiet 模式、首行、解析失败；不排除关键字，不跳过注释，不扩展到字段或下标。
Full REPL 没有这项兜底，与固定 C++ 的两条入口边界相同。

更严重的独立问题是执行错误与解析错误共用 `CliError`，外层对所有错误文本搜索
`<eof>`、`to close` 等字样。合法程序 `error('<eof>', 0)` 因此被当成未完成源码，
下一行被追加后又从头执行旧程序，后续命令到不了，已发生的副作用还可能重复。

交互路径现在保留 `ReplError::Syntax(ParseError)` 与 `Terminal(CliError)`。
只有解析失败可以续行；代码生成、Runtime、发布和宿主访问错误直接报告并重置
输入状态。诊断保持原始字节，编译成功的 Proto 在执行成功或失败后仍移除显式根。

续行检查按固定 C++ 的五类解析消息执行，并将 Rust lexer 的三种不同拼写按错误
前缀对应。移除泛化的 `to close`：确定的非 EOF 语法错误不能吞掉下一条命令。
这些判断不是任意错误文本的重新分类。

## 证据与验收边界

原诊断使用 `bf813…` 快照。本轮先用上一工作包最终 Debug
`54d17dd86c72bfece332026008489d5263d4c5c81ee4f463ac13fa2cc2efc839`
重跑六个场景 × 三个引擎，stdout/stderr 原始字节及退出状态与旧观察全部相同，
记录在 `target/main-repl-evidence/current-before/observations.json`。
未修改的原版 main 红基线也重跑并保留为 `official-main-before.json`。

首轮实现后，六场景 replay 中四个原 Rust 问题按预期变化，两个其余场景保持不变。
既有 Debug REPL 8/8、CLI 18/18 合同通过。该轮原版 main 已越过所有交互测试段，
停在第 135 行首次 `string.dump`，仍返回失败，没有到达完成标记。
产物为 `first-after/observations.json` 与 `official-main-first-after.json`。
扩展 `repl_continuation` 最终 19/19 通过，包含后来补充的 REPL/batch/protected
loader break 回归。联合冻结后完整 M0/M1 基础门通过，普通测试共 1,056 项；
最终 Debug/Release 的 CLI 18/18、REPL 8/8 均通过。
最终原版 main 两种构建仍停在 dump，分别保存在
`target/repl-break-bytecode-evidence/official-main-debug-final.json` 与
`official-main-release-final.json`。联合源码身份及完整验收见
[当前整改状态](2026-09-12-remediation.md)，不继承前一提交的远程 CI 结论。

## 保留的边界

- `t.x` 换行 `= 10`：固定 C++ 与 Rust 均不续行，官方 Lua 可以续行。
  本次不把 stock 的更宽行为套入固定 C++ 的项目合同。
- 两端 lexer 都只跳过 space/tab/CR/LF；Quiet helper 另裁掉 VT/FF。
  裸名称加 VT/FF 的完整源码处理要区分 helper 和 lexer，不声称支持所有空白。
- `<eof>` 等 marker 在解析错误中的宽泛匹配沿用固定 C++；运行错误已从此路径隔离。
- 探针移除 `LUA_INIT`，没有验证其支持。Full 的物理终端编辑/中断不由管道测试证明。
- 固定 C++ 完整 main 通过；官方 Windows 在带点临时模块名的 require 阶段失败。
  Rust 的剩余 main 失败是 binary chunk 依赖，与 `calls.lua` 一起保留到 M3。

该批验收后的首项 [嵌套 codegen 错误传播](2026-09-12-nested-codegen-remediation.md)
现已单独完成。下一步处理 Windows files 行为、其余诊断入口与有界大负载，再按
[M3 合同设计](../rust_migration/m3_contract_design.md) 推进格式 reader/verifier、
原子发布和 serializer。不会因为修复交互段就把整份 main 纳入严格通过子集。
