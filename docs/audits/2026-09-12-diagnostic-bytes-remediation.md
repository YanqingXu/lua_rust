# 错误诊断原始字节整改

本工作包接续 `4024f9e` 后的 debug 工作区，固定 C++ oracle 仍为
`87c15e69ceb94eb74e28226ccbefb7e196635711`，第二 oracle 为官方 Lua 5.1.5。
没有修改 vendored `errors.lua`、来源清单或参考实现版本。

## 问题与实现

官方 `errors.lua:196` 原先因错误消息把 raw `0xFF` 改成四个 ASCII 字符
`\xff` 失败。Lexer 已保存原始 token；损失发生在 parser 的 near token 渲染，
以及默认 loadstring 源名的 chunk-id 渲染。仅在最终输出反转义会破坏用户真实
输入的反斜杠文本，不能修复这些结构信息的丢失。

`ParseError`、`CodegenError` 与 `RuntimeError` 现在保存独立的 owned 字节消息，
通过 `message_bytes()` 读取。现有 `message: String` / Display 保留为宿主可读
视图；它们不再承担 Lua 错误值传输。Parser near 直接拼原 lexeme，构造、转换、
Runtime 来源定位均保留原始字节。

VM 编译、loadstring、loadfile、dofile、reader continuation 和 package 编译错误
用字节消息发布到 Lua 栈。Chunk id 保留固定 C++ 的 `@` / `=` 标记规则、源码
片段引号与反斜杠规则、60 字节片段预算；高位字节直接保留。
VM 的 call/index/arithmetic 操作数描述也以字节保存 field 等名称。

`lua_app` CLI 的编译、执行、require、脚本及 REPL 错误使用本地字节错误类型，一直传到
原始 writer；不经 Display 再编码。Runtime 在原错误值仍有效时复制显示字节，
CLI 不需要在回收或关闭之后重新解引用错误对象。
独立 `lua_bytecode` 工具在本工作包冻结时仍使用宿主 String 视图；其后续修复与
单独的 NUL 输出边界见 [bytecode 工具诊断整改](2026-09-12-bytecode-diagnostic-remediation.md)。

显式错误对象仍与 VM 诊断分开：`error(raw, 0)` 原字节不变，table 保留 identity，
`error(23, 0)` 保留 number。两个 oracle 对 `error(23)` 默认层级共同要求带位置的
string；该入口现在同时处理数字与字符串的定位，未把其他对象统一字符串化。

## 验证与证据

开始前已对上一轮最终 Debug binary
`c54ba3fd90d36d293043d4f7fbef04b6cfcefc686f1995ca4ad1da2366cb9713`
重跑 19 个小探针 × 3 引擎，原始输出和退出码与旧 `fc06…` Rust 快照逐项一致。
本轮使用这些新运行作为红基线，记录在
`target/errors-byte-evidence/current-before/observations.json`。

回归覆盖：

- 编译器原始 near 字节、Name/Number/string 的原 lexeme、EOF 与 fallback、
  字面 ASCII `\xff` 与真实 `0xFF` 的区分，以及 ParseError→CodegenError 转换。
- Lua 加载器、reader 分片、源名与 token 同时含高位字节、loadfile/dofile、
  runtime field 名、显式错误类型与对象 identity。
- CLI 独立子进程直接检查 stderr 原始字节，异常退出、超时和意外输出均失败。
- 两个 Runtime/Miri 用例：加载器与 protected call 的真实发布路径，以及
  Runtime close/drop 后才首次读取错误消息，仍保留 `0xFF`、NUL 与后缀。

第一轮未经修改的官方 `errors.lua` 已在 Debug 构建及两个 oracle 中完整完成，
产物为 `target/errors-byte-evidence/official-errors-first.json`。
最终验证使用修正 Clippy 后的相同冻结工作区：

- 完整 M0：74,375 ms，`fullGatePassed=true`，0 hard failure；83 match / 9 debt。
- 完整 M1：50,465 ms，`foundationPassed=true`，0 hard failure。质量门包含
  fmt、all-targets Clippy、Debug/Release、doctests、warning-free rustdoc 与 audit，
  无跳过。普通测试清单 1,027 项，无 ignored；doctests 另行执行。
- Debug 严格子集十项（constructs、db、errors、literals、locals、math、nextvar、pm、
  strings、vararg）× 三引擎全部完成；Release 的 db/errors × 三引擎全部完成。
  CI 的严格子集已加入原版 `errors.lua`，新字节边界 Miri target 也已接入。

完整门产物为 `m0-final.json`、`m1-final.json`；普通清单为 `test-list.log`。
M1 的七份步骤 JSON 已另存 `m1-step-artifacts/` 并记录 SHA，避免后续门覆盖默认路径。
原版官方执行产物为 `official-required-debug-final.json` 和
`official-db-errors-release-final.json`，包含原始源码、引擎快照和完成标记。
单个官方文件通过不代表所有错误正文与两个参考实现完全相同；本轮没有重跑完整
19 项，历史 11/19 快照不改写。

首次完整 M1 只在 CLI 两处 `needless_borrows_for_generic_args` 失败；移除多余
借用后重新运行上述完整门。首次失败保留在 `m1-initial-failed.json/.log`，未覆盖。

默认 Miri 已在冻结后的生产实现上执行两项真实源码回归，2/2 通过，共 282.20 秒。
命令为 `cargo +nightly-2026-07-29 miri test -p lua_stdlib --test error_diagnostic_boundary`，
未设置 `MIRIFLAGS`；日志 `target/errors-byte-evidence/miri-error-bytes.log`。
测试执行到 Runtime close、drop 和随后首次消息读取；对象、托管字节和资源
关闭错误的归零断言均已执行。

本轮 Cargo、配置、源码、fixtures、tools 与 CI 共 369 个最终输入文件已冻结：
`target/errors-byte-evidence/source-verified.zip`，清单 SHA-256
`091db23ba5ea1de4c426d5ccb35cbada21574ae195a0d8379c466ebc5e6d15b4`。
最初 `source-final.zip` 的 `d3a6…` 清单保留为 Clippy 修正前的输入；默认 Miri
在该轮执行，此后只有 CLI 两处借用写法变化，Miri target 不编译 CLI。
该快照包括上一轮尚未提交的 debug 修复。测试由相同的未变化工作区构建，归档用于
保存原始输入身份，不声称从 ZIP 重新构建。该验收时远程 CI 只验证过 `4024f9e`；
本批现随联合批次提交，远程验证须按新提交 SHA 单独确认。

最终 Rust 二进制 SHA-256：

- Debug：`54d17dd86c72bfece332026008489d5263d4c5c81ee4f463ac13fa2cc2efc839`。
- Release：`bf193d0f137105f67be4c002fa76d9aeeb4dc780d8b713082805b34131c29b4b`。

## 保留的分歧与后续

两个 oracle 的语法错误正文不同；本轮共有回归锁定原始字节与类型，不把不同正文
归一化成“完全一致”。另有两项明确分歧：

- 固定 C++ 的 reader 语法错误可能丢弃显式 chunk name，官方 Lua 保留它。
  共有 reader 回归用 ASCII 名称，高位 chunk name 的合同由 loadstring 单独验证。
- 固定 C++ `package` 在捕获 parser/codegen 失败后只返回模块加载失败的通用信息，
  官方 Lua 保留语法诊断。Rust 原先已交付语法诊断，本轮保留该既有行为并防止字节
  损失；require 回归属于 Rust 的字节传输合同，不是新批准的 C++ deviation。

上述分歧的完整原始输出在 `current-before/reader-name-conflict.json` 和
`require-extension/oracle-probes.json`。未因这些分歧改变 oracle pin 或比较规则。
非法数字 `1e+` 的 token 边界是独立词法问题；CLI 的 `LUA_INIT` 功能尚未实现。
接下来推进 `main.lua` 的交互续读，再处理平台 IO 与大负载，M3 binary chunk / C API
继续保留其前置验收条件。
