---
status: provisional
milestone: M2
last_updated: 2026-08-12
evidence_snapshot: target/compatibility/m218-bytecode-m0-final/non-official.json
bytecode_evidence_snapshot: target/compatibility/m2-compiler-limits-bytecode-131-final/report.json
bytecode_integration_snapshot: target/compatibility/bytecode-integration-bounded-12-after-coroutine/report.json
vm_trace_evidence_snapshot: target/compatibility/m2-callback-final-pinned/parity-runner-self-test/vm-trace-complete-lifecycle/report.json
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# M2 non-official 差异聚类与开发 backlog

## 结论

当前可审计结论是 **92 个已执行 case 中 runner status match 81、difference 11**。
`stdlib.iolib` 的 Windows text seek/tell 与 `io.popen` 已关闭并移入 match；
manifest normalization 后全通道相等为 82/92，direct raw stream/exit 仍为 78/92；
三项差额是 manifest 已显式登记的 executable identity。多出的
`integration.env-metatable-loader` 两端
stdout/stderr/exit/side effect 完全一致，但都以 1 退出，违反 manifest `expected_exit=0`，
因此正式 status 仍是 difference。其余 10 项是 normalization 后仍存在的实现间差异，且全部
只在 stdout；stderr 与已声明 side effect 均为 92/92 一致。没有 timeout、runner error 或
两实现 exit-code 差异。

本轮以固定 C++ raw-byte 探针关闭了 Windows CRT text-mode 主根因：标准输出、标准错误、
已有 `CRLF`、高位/NUL 字节，以及 `"w"`/`"wb"` 文件副作用均逐字节对齐。没有扩大
runner normalization，正式 artifact 保留剩余全部差异。

M2.7 已接入真实 JSONL Snapshot/Diff、固定 C++ evidence adapter、Rust
`lua_app --trace file`/`--trace-diff file` 与同一 12-case 代表集；两种 Basic 模式跨实现均为
12/12 passed、0 difference、0 infrastructure failure，`stackTop`/`changedRegisters` 进入完整
M0 硬门。Complete 核心 4-case manifest 已在 Snapshot 与 Diff 两种模式各 4/4 零差异，
双端精确观察 instruction/call/return/yield/resume/upvalue-open/upvalue-close/error，旧的
missing-evidence 债务已经关闭。随后 callback metamethod/debug-hook 的 Snapshot/Diff
双 2-case 也已零差异。固定 C++ adapter 现通过配置 allocator 捕获 `LuaState` 析构后的
精确 deallocation，并与 Rust 各自通过 3 captured open/2 GC close 的 fail-closed producer
自检；M2.7 lifecycle completed-local，更广 M2.3 trace corpus 仍开放。

随后 M2.1 compiler limits/error 切片关闭了超长跳转与匿名 child codegen 的用户可达 panic，
并完成递归/嵌套、locals/upvalues、lvalue/method、register/constant 极限矩阵。该切片不改变
92-case process debt：本轮 IO 后 fresh status 为 81/11。

11 项 runner status debt 的主根因分区如下。每个 case 只在主分区中计数一次；同一 case 的次级根因
会在详情中保留。

| 主分区 | 数量 | 最小现有代表 | 主要归属 | 建议优先级 |
|---|---:|---|---|---|
| userdata 的 `print` 表示 | 2 | `regressions.move-bug` | base + IO userdata | P1 |
| number formatting + arithmetic error location | 1 | `tables.metamethod-arith` | base number renderer + VM error object | P1 |
| GC control 与内存计账 | 7 | `stdlib.collectgarbage-count` | runtime GC + base | P0，依赖 M1 GC |
| 双方共同 expected-exit failure | 1 | `integration.env-metatable-loader` | fixture intent + loader environment | P1 |
| **合计** | **11** |  |  |  |

首要判断是：进程 stdout 看起来只差换行，不代表生成的 Proto 相同。fresh
representative bytecode artifact 已在受审计 C++ byte-safe constant/local sidecar 接入后达到
2/2 passed、0 difference；这只关闭两个简单 case，不代表 nested compiler parity
完成。M2 仍必须并行推进 process observable 与 bytecode/VM trace，不能用前者的
“归一化后相等”代替后者。

2026-08-11 至 2026-08-12 后续纵切片更新（不改写上述 M0 历史快照）：CALL statement
result convention、contiguous RETURN、constant interning/order 与 max-stack 已修；
Rust bytecode JSON schema v2 也补齐无损 constant、递归 sub-Proto、function/
line/local/upvalue evidence。固定 C++ checkout 外的只读 sidecar 现从
`Proto` 输出 byte-preserving constant/local metadata，并与 pristine full-text
printer exact-path 合并；schema 2 对 string 使用 byte envelope、对 number 使用 bits，且已通过
embedded-NUL fail-closed 自检。原两个代表 case 已完全一致。扩展
`test_closure_pipeline.lua` 后，开放式表字段容量、nested assignment lvalue freezing
与 arithmetic relocatable result、for-loop iteration-scope `CLOSE`、boolean-only
short-circuit jump list、root-scope `CLOSE` 和 lineInfo 已逐根因修复。103 行
`test_closure_pipeline.lua` 的根 Proto 与全部 child 现为完整 structured 1/1 passed、
0 difference。随后又修复 helper table/concat/line scheduling、member-call 与 local
multi-return frame、immediate RK constant、nested-call LineGuard；随后 24-case 扩展又修复
parenthesized condition jump-list、`if endLine` exit jump、`repeat` value-condition lowering，
并加固单元素 child line-info 解析。继续扩到 36-case 后，又同步 RETURN 临时基址、局部 nil
初始化活性、legacy vararg flags、`do` CLOSE 行号及 while/break 双出口 CLOSE。继续扩到 48-case
后，又补齐 byte-safe constant evidence、nonlocal function closure temporary、break post-close
target、`if` CLOSE 行号、统一词法 block cleanup 与 parenthesized-immediate folding。继续扩到
60-case 官方压力语料后，又同步 table array call field 的 free-register 精确回收与
`not not <literal>` 真值折叠。继续扩到 72-case 后，剩余 8 个 vendored official Lua 文件与
4 个复杂 non-official 文件均一次通过；随后盘点全部 132 个 Lua 文件，确认 131 个有效源码与
唯一刻意非法的 `basic/syntax_error.lua`。full-valid exact-set manifest 的 131/131 structured
比较为零差异，并覆盖全部 24 个 vendored official 文件；筛查同时修复 Rust bytecode CLI
语法错误打印诊断却返回 0 的问题。5-case integration corpus 与全量 131-case valid structured
corpus 均为零差异；131-case 本身覆盖 36/38 opcode，
即固定 C++ 可执行合同中的 36/36
frontend-produced opcode；`NOT/TESTSET` 明确 intentionally-not-produced。该结果仍不能外推为
全部 compiler/VM parity 完成；当前仓库的有效 corpus 扩展入口已耗尽。显式 compiler
error/limit parity 现已 completed-local，下一入口转为 process observable 与真实 VM trace。

2026-08-12 error-envelope 纵切片又统一了 C++ `makeLuaChunkId` renderer、`lua_app.exe:`
program prefix、Windows host diagnostic CRLF 和 CALL nil 的活动 Proto/line provenance。
`basic.syntax-error` 现为 raw match；`integration.env-metatable-loader` 的 stderr raw bytes
完全一致；`stdlib.coroutine-errors` 不再泄漏内部 `@`。后两项只剩 stdout LF/CRLF，
因此 error/source/line 主分区由 4 项降为 1 项，raw 状态变为 5/87。随后 M2.16
text-mode 纵切片关闭了 75 项原有 EOL 主根因（其中 3 项仍保留预期 executable identity
raw 差异）和两个文件副作用，正式 status
变为 80/12，manifest-normalized 全通道相等为 81/92、direct raw stream/exit 为 78/92。
M2.1 随后的 fresh gate 保持相同计数，并新增 8-case raw error matrix、5-family long-control
matrix、300-register structured bytecode 与 262145-constant process probe；两个原 Rust panic
均已变成与 C++ 完全相同的 exit/stderr。

2026-08-12 Complete lifecycle 纵切片又完成 Rust coroutine error frame、protected unwind/
yield barrier、native TAILCALL、Automatic-GC continuation、callback event barrier，以及 C++
active-state/upvalue/stackTop adapter 对齐。Rust trace state 全量改用带 generation 的
`StateHandle`，arena retirement 在 generation advance 前交付 close 并清理 state cache；
重复 create/suspend/collect 回归不再跨世代抑制 open。核心 Snapshot/Diff 双 4-case 均零差异；
callback RETURN/caller `stackTop`、debug CALL depth 与 caller-upvalue changed-register 随后修复，
callback 双模式 2-case 也零差异；C++ allocator retirement 通知与双 producer 生命周期自检
随后关闭了 GC trace-state oracle 缺口。

## 1. 证据范围与判定口径

### 1.1 使用的快照

主要证据：

- `target/compatibility/m2-compiler-limits-final-m0-pinned/non-official.json`，生成时间
  2026-08-12（本地）；selected 101、executed 92、helper 9、status match 80、
  difference 12、manifest-normalized 全通道一致 81、direct raw stream/exit 一致 78、
  runner error/timeout 0；
- fresh Rust executable SHA-256 为
  `f973046d168999dd4e91f5cd01529800d3978b037663e860153fcf3d25de9b5f`，固定 C++
  executable SHA-256 为
  `bde546c0fac16cadf9dd3e6b161bf1ed84ba210ea3b2be1320da34cd34d63695`；runner 0.1.0；
- 与上一份 5/87 error-envelope artifact 相比，75 个 EOL 根因已关闭；其中 3 项仍保留
  manifest 已登记的 executable identity raw 差异。
  `integration.env-metatable-loader` 也成为 raw 全通道一致；它只因双方共同违反
  `expected_exit=0` 保留 status difference。两个原文件副作用差异均已 exact-byte 关闭；

历史证据：

- `target/compatibility/non-official.json`，生成时间
  2026-07-26 15:41:11 +08:00；
- `target/compatibility/non-official-fresh-oracles.json`，生成时间
  2026-07-26 15:39:38 +08:00；
- 两份报告都是 selected 101、executed 92、helper skipped 9、match 4、
  difference 88、runner error 0、timeout 0，88 个 difference ID 完全相同；
- C++ executable SHA-256 为
  `782f193c4d9ebed7c3d22a5c0ab08a0a677f551e61cfa7a8402403ec78a69f79`，
  对应固定提交 `87c15e69ceb94eb74e28226ccbefb7e196635711`；
- 原 M0 Rust executable SHA-256 为
  `b8bfff298a655c93c78797a2cf29b8d92880c5ac72d98634e3e6961cba0440df`。

这两个历史 non-official artifact 早于当前工作树中的 ByteString、IO scoped-access、
runtime ownership 与 GC/root 迁移。fresh artifact 已取代其作为当前判定来源；旧 artifact
仅用于解释 case 移动。

### 1.2 三层比较

本文明确分开三种口径：

1. **L0 raw gate**：直接比较 exit、timeout、stdout bytes、stderr bytes 和已声明
   的 side effects，并同时检查 manifest expected exit。这是正式状态来源，结果为
   80 status match / 12 difference；忽略 expected-exit 断言时有 81/92 两实现
   manifest-normalized 全通道一致，direct raw stream/exit 则为 78/92。
2. **L1 manifest normalization**：只使用 fixture 已声明的 literal/regex 规则。
   例如 runtime arg case 把各自的 `{{executable_slash}}` 替换成
   `<lua-executable>`。报告仍保留 raw bytes 和 difference。
3. **L2 attribution-only normalization**：旧 artifact 曾在 L0 或 L1 结果上临时把
   `\r\n` 视为 `\n`，仅用于定位 Windows text-mode 根因。当前 fresh artifact 不再需要
   L2 来解释 EOL；实现本身已按固定 C++ raw bytes 对齐，L2 从未写回 artifact。

当前 11 个 normalization 后实现间差异都是 stdout difference；direct raw 另有
3 个已登记 executable identity。stderr 与已声明 side effect 均为 92/92 一致。
`integration.env-metatable-loader` 是第 81 个 normalized 全通道一致 case，但其 raw
channels 本身也完全相等；expected-exit 断言仍失败，所以不能并入 80 个正式 status match。

### 1.3 不能从当前 artifact 推导的结论

- 不能说 80 个 status match 覆盖了对应功能；process observable 相同仍不等于内部路径完整；
- 不能从 process 输出相同推导 opcode、register allocation、constant order、
  Proto debug metadata 或 VM trace 相同；
- 不能从 7 个 GC 数字 case 推导 weak table、finalizer、resurrection 或
  shutdown 的具体根因；它们只证明 GC API 的可观察值不同；
- 不能从 `stdlib.iolib` 一个 case 推导所有未实现函数。该 case 只直接证明
  `io.popen` 缺失；`string.dump`、native `package.loadlib`、C API 等缺口由
  其他 artifact 和 deviation 记录覆盖。

## 2. EOL 与路径噪声

### 2.1 已关闭：72 个主归因为 LF/CRLF 的 case

`basic.print` 是最小证据：

| Engine | stdout hex |
|---|---|
| Rust（旧） | `48656c6c6f0a` (`Hello\n`) |
| Rust（fresh） | `48656c6c6f0d0a` (`Hello\r\n`) |
| C++ | `48656c6c6f0d0a` (`Hello\r\n`) |

旧 Rust 的 `crates/lua_stdlib/src/base.rs` 把 `b'\n'` 直接写到 binary `Write` 边界；
固定 C++ 的 `std::fputs` 经 Windows CRT text stream 写成 CRLF。fresh 实现新增集中
`lua_stdlib::host_io`，base、IO 与 CLI 共用 byte-oriented text writer。固定 C++ 探针已
锁定 `LF -> CRLF`、已有 `CRLF -> CRCRLF`、高位/NUL 保持，以及 stdout/stderr/text file
一致、binary file 不转换。以下 72 项的 EOL 根因现已关闭。

72 项按 fixture 域列出如下：

- alien-signals（1）：
  `alien-signals.example`
- basic（3）：
  `basic.arithmetic`、`basic.basic`、`basic.print`
- bytecode（2）：
  `bytecode.debug`、`bytecode.basic`
- control-flow（19）：
  `control-flow.break-counter`、`control-flow.break-for`、
  `control-flow.break-minimal`、`control-flow.break-nested`、
  `control-flow.break-simple`、`control-flow.for-simple`、
  `control-flow.if-basic`、`control-flow.if-comparison`、
  `control-flow.if-else`、`control-flow.if-elseif`、
  `control-flow.if-eq-simple`、`control-flow.if-in-while`、
  `control-flow.if-logic`、`control-flow.if-nested`、
  `control-flow.if-simple`、`control-flow.if-truthy`、
  `control-flow.if-smoke`、`control-flow.simple-if`、`control-flow.simple-if-while`
- functions（9）：
  `functions.factorial`、`functions.closure-simple`、
  `functions.direct-call`、`functions.multiret`、
  `functions.simple-call`、`functions.table-constructor-multiret`、
  `functions.vararg-simple`、`functions.vararg-table`、
  `functions.vararg`
- integration（4）：
  `integration.closure-pipeline`、`integration.coroutine-scheduler`、
  `integration.env-metatable-loader`、
  `integration.loadfile-dofile-workflow`
- regressions（9）：
  `regressions.call-pipeline`、`regressions.logical-return-combo`、
  `regressions.lvalue-matrix`、`regressions.lvalue-pipeline`、
  `regressions.multiret-edges`、
  `regressions.short-circuit-materialization`、
  `regressions.upvalue-close`、`regressions.value-pipeline`、
  `regressions.vm-self-limitation`
- runtime（6）：
  `runtime.arg-index`、`runtime.arg-table`、`runtime.arg-very-simple`、
  `runtime.arg`、`runtime.global-access`、`runtime.globals`
- stdlib（14）：
  `stdlib.coroutine-basic`、`stdlib.coroutine-closures`、
  `stdlib.coroutine-errors`、`stdlib.coroutine-generators`、`stdlib.coroutine-status`、
  `stdlib.coroutine-values`、`stdlib.debug`、`stdlib.getfenv`、
  `stdlib.io-debug`、`stdlib.io-minimal`、`stdlib.io-type`、
  `stdlib.iolib-simple`、`stdlib.setfenv-detailed`、`stdlib.setfenv`
- steps（3）：
  `steps.basic`、`steps.logic`、`steps.table`
- tables（2）：
  `tables.access`、`tables.zero-key`

已执行验收：

1. 未给 case 增加 CRLF normalization；修复发生在 host text-output 边界。
2. `io.write`、stderr、base `print` 覆盖无换行、LF、已有 CRLF、NUL 与高位字节。
3. `"w"` 与 `"wb"` 同源内容分别得到 C++ exact physical bytes；92 个已执行 case 的
   已登记 side effect 现为 92/92 相等。
4. 直接 raw stream/exit 仍保留下节 3 个 executable identity 差异，不把它们误报为
   产品 byte 相等。

### 2.2 三个 executable identity case

`runtime.arg-minimal` 的唯一非 EOL raw 差异是：

- Rust `arg[-1]`：
  `.../target/x86_64-pc-windows-msvc/debug/lua_app.exe`
- C++ `arg[-1]`：
  `.../target/oracles/lua_cpp/build/Release/lua_app.exe`

两个程序不可能拥有相同真实 executable path。fixture 已为 runtime arg cases
声明 `{{executable}}` 和 `{{executable_slash}}` 的 per-engine literal
normalization；在 artifact 的 normalized output 中，两边都成为
`<lua-executable>`。`runtime.arg-minimal`、`runtime.arg-negative`、
`runtime.arg-simple` 的 EOL 已关闭；三项在 manifest-normalized 比较中均为 match，直接
raw 口径只保留不可消除的真实 executable path identity。

建议把这三项留在“harness identity noise”而非产品修复队列，同时保留独立 CLI
结构测试，精确断言 `arg[-1]`、`arg[0]`、正参数、`-e`、`--` 的索引和值。
路径 normalization 不能替代这些结构断言。

## 3. CLI 与 source-byte 边界

### 3.1 `control-flow.simple-if`

旧 artifact 的同一行：

- Rust：`Lua if è¯­å¥å¨é¢æµè¯`
- C++：`Lua if 语句全面测试`

exit code 都是 0，控制流结果没有其它已观察差异；差异出现在 UTF-8 文本被二次
编码后的输出。旧实现中 Lua string 经过 Rust text/legacy-byte 混合路径，符合
该 mojibake 形态。

当前 `crates/lua_app/src/main.rs` 已用 `fs::read` 读取 source bytes，compiler
也已接收 byte source，`GcString` 已迁移到 canonical `ByteString`。2026-08-11
error-envelope artifact 中 Rust/C++ 的中文内容 bytes 已一致，仅保留 LF/CRLF；M2.16
fresh artifact 又关闭了 EOL，因此该 case 现为正式 status match，M1/M2 修复形成
regression verification。

后续仍应把边界缩成独立 probe：

1. 新增或复用最小 probe：源码只包含
   `print("中文✓")`，同时输出 `#s` 和每个 `string.byte`；
2. 在 CLI 文件、stdin、`load` reader、`loadfile` 四个入口运行；
3. 比较 raw stdout hex，不使用 Unicode text normalization；
4. 保留 CLI 文件、stdin、`load` reader、`loadfile` 四入口，防止后续迁移回退。

该项不再是活跃根因，fresh runner 已按同平台 raw EOL 合同通过。

## 4. 编译器与 VM

### 4.1 process case 暴露的真实问题：error provenance

进程 corpus 中大多数控制流、closure、multiret 和 regression case 在 L2
观察下输出相同，但这只证明最终脚本输出相同。当前仍明确暴露 VM error provenance 问题的是：

- `tables.metamethod-arith`
  - Rust error object 没有 source/line；
  - C++ 包含
    `.../test_metamethod_arith.lua:141: attempt to perform arithmetic ...`。

Rust 的 `crates/lua_vm/src/execute.rs` 已有 `runtime_error_at(proto, pc, ...)`，CALL 非函数
分支已接入；算术/一元错误等分支仍有直接构造 `RuntimeError::new(...)` 的路径。
固定 C++ 的 `src/vm/state/lua_state.cpp` 通过 `makeLuaChunkId` 和
`runtimeErrorWithLocation` 从活动 frame 生成 source/line，并去掉 chunk name
的 `@`/`=` 标记。

建议 P0：

1. 统一 VM error constructor，所有 opcode、metamethod、host-call failure 都带
   error object、source kind、line 与 category；
2. 明确 chunk-id rendering：`@file` 显示为 `file`，`=name` 显示为 `name`，
   string chunk 使用 C++ 相同摘要；
3. `pcall`、`xpcall`、coroutine resume 和顶层 CLI 只负责各自 envelope，不重复
   或丢失 location；
4. 为 CALL-nil、arith-table、metamethod error、load reader chunk 各建一个
   3–8 行最小 case。

### 4.2 不能被 EOL 聚类隐藏的 bytecode 债务

`target/compatibility/bytecode-representative/report.json` 是独立证据：

| Case | 结果 | 结构化 difference |
|---|---|---:|
| `test_bytecode.lua` | passed | 0 |
| `test_bytecode_debug.lua` | passed | 0 |

2026-08-12 fresh artifact 中两例的 opcode、constant、max stack、Proto/function/line/upvalue/local
metadata 均一致。C++ byte-safe constant/local 证据链为：

- pristine `lua_bytecode` 保持固定 checkout 原行为，原 executable SHA 不变；
- checkout 外 `tools/cpp_bytecode_evidence` 链接固定 checkout 安装的 `LuaCpp::Lua`，只读
  `Proto` const accessor，并输出 schema/commit/path、constant byte/bits envelope 与 LocVar metadata；
- comparator 要求 sidecar 进程成功、schema 2、commit 精确匹配、每个 constant/local 的值、
  name 与 metadata 一致、Proto path 双向 exact coverage，任何缺口均 fail closed；
- sidecar executable SHA 为
  `6ad885e1164d7039c7fcab9d7b4181195534ddb222d7d4c343ebf93c55cc0e47`，source/CMake
  SHA 已写入 `target/compatibility/cpp-oracle-build-byte-safe-constants.json`。

因此两个简单 `bytecode.basic` case 可以关闭，但不能据此外推
compiler-parity completed。下一顺序：

1. 已完成：为固定 C++ Proto 建立受审计 byte-safe constant/local sidecar evidence，null 或
   embedded-NUL 截断不再可能被当作 pass；
2. 已完成：首个 nested 分歧是 `0.children[0]` PC 0 的 `NEWTABLE B=0/1`，已用
   open vararg/call 两个最小 case 修复固定 array hint 计数；
3. 已完成：`0.children[1].children[0]` 的 assignment/register scheduling；按固定 C++
   顺序先冻结全部 lvalue、再求 RHS、最后逆序 store，并补齐 local fast path/swap；随后将
   arithmetic result 改为由 consumer patch 的 relocatable register。`remember` 从 13/15
   收敛为 13/13，完整 pipeline closure 也达到 34/34 word-identical；
4. 已完成：`0.children[2]` numeric-for scope `CLOSE`。固定 C++ 对非空 numeric-for
   body 无条件发 `CLOSE(base+3)`，generic-for 对可见循环变量也无条件发；Rust 已同步
   空/非空 body 合同，`fold` 从 19/18 收敛为 19/19；
5. 已完成：根 Proto boolean-only `and/or` 复刻 C++ `producesBoolean` jump-list 管线，
   同时保留普通 Lua `a and b` 返回原值的通道；root captured locals 在隐式 RETURN 前
   发 `CLOSE A=0`，parent/child line state、隐式 RETURN last-line 与 while synthetic line 0
   也已对齐；
6. 已完成：首批 bounded 扩展修复 helper table/concat/line scheduling、member-call temporary
   frame、local call multi-return frame、immediate RK constant 与 nested-call LineGuard；
   integration corpus 5/5、显式 bounded structured corpus 12/12，均为零差异；
7. 已完成：新增动态 `UNM/LEN` 与 value-form `not` differential case；固定 C++ coverage
   contract 确认 `NOT/TESTSET` intentionally-not-produced。13-case corpus 与 integration
   corpus 并集达到 36/36 frontend-produced opcode；
8. 已完成：扩到 24-case，并修复 parenthesized condition、`if endLine`、`repeat` value lowering；
   comparator 新增 recursive singleton line-info 自检。24/24 为零差异；
9. 已完成：扩到 36-case，并修复 RETURN 临时基址、局部 nil 初始化活性、legacy vararg flags、
   `do` CLOSE 行号及 while/break 双出口 CLOSE。36/36 为零差异；
10. 已完成：扩到 48-case，并修复 byte-safe constant evidence、nonlocal function closure
    temporary、break post-close target、`if` CLOSE 行号、词法 block cleanup 与括号 immediate
    folding。48/48 为零差异；
11. 已完成：扩到 60-case 官方压力语料，并修复 table array call field 临时寄存器回收与
    双重 `not` literal truthiness folding。60/60 为零差异；
12. 已完成：扩到 72-case，纳入全部 24 个 vendored official Lua 文件；新增 12 个复杂文件
    初次即为零差异，72/72 通过；
13. 已完成：建立当前仓库 full-valid exact-set manifest；131/131 个有效 Lua 源码全部为
    structured 零差异，并修复非法源码 Rust bytecode CLI 的零退出状态；
14. 已完成：显式 compiler error/limit parity；codegen 无显式 `panic!`，long jump、nested
    invalid lvalue 与 malformed method AST 均返回结构化错误；parser/CLI/register/constant
    极限证据与固定 C++ 对齐；
15. 已完成：真实 Snapshot/Basic trace 第一切片，12-case 代表集零差异并进入 M0 硬门；
16. 已完成：真实 Diff/Basic、stack top 与 changed-register，12-case 代表集零差异并进入 M0
    硬门；
17. 已完成：真实 CALL/RETURN 与顶层 error value/category；
18. 已完成：Complete 核心 upvalue/yield/resume/protected-error 四 fixture，Snapshot/Diff
    双 4/4 零差异并覆盖全部八类事件；
19. 已完成：C++ adapter allocator retirement hook 与 Rust `StateHandle` generation 各自通过
    3-open/2-GC-close producer 自检，M2.7 lifecycle 标记 completed-local；
20. 已完成：M2.16 CLI 双实现 18/18，覆盖参数/stdin/actions/hash-bang/source-name、稳定文件
    错误与 REPL error recovery；binary chunk 明确路由 M3；
21. 已完成：M2.17 REPL 双实现 8/8，覆盖 banner/help/history/goodbye、多行/不完整输入、
    错误恢复与 stack reset、表达式、meta command 和 completion；
22. 已完成：M2.18 bytecode viewer 15/15，固定 C++ compact/full/cfg/diff 与错误合同
    12/12 逐字节一致，Rust JSON/strict-format 扩展 3/3。下一主任务为 M2.9 arithmetic error provenance。

当前执行结果：

- `target/compatibility/bytecode-representative/report.json` 中两例 2/2 passed，左右
  `localNames=true`，每例 0 difference；
- `target/compatibility/bytecode-closure-full-parity/report.json` 为 1/1 passed、
  0 difference、0 infrastructure failure；根 Proto 198/198，6 个 child 分别为
  10/10、14/14、13/13、34/34、19/19、5/5，instruction/constant/function/line/local/
  upvalue evidence 全部一致；
- `target/compatibility/bytecode-integration-bounded-12-after-coroutine/report.json` 为
  5/5 passed、0 difference、0 infrastructure failure，共 23 个 Proto、1206 条 instruction、
  290 个 constant，覆盖 29/38 opcode；
- `target/compatibility/bytecode-structured-bounded-24-after-repeat-value/report.json` 为
  24/24 passed、0 difference、0 infrastructure failure，共 107 个 Proto、3295 条 instruction、
  765 个 constant，覆盖 36/38 opcode；正向 manifest 为
  `tests/compatibility/bytecode-structured-bounded-24.json`；
- `target/compatibility/bytecode-structured-bounded-36-final/report.json` 为 36/36 passed、
  0 difference、0 infrastructure failure，共 235 个 Proto、7673 条 instruction、1746 个
  constant，覆盖 36/38 opcode；正向 manifest 为
  `tests/compatibility/bytecode-structured-bounded-36.json`；
- `target/compatibility/bytecode-structured-bounded-48-final/report.json` 为 48/48 passed、
  0 difference、0 infrastructure failure，共 393 个 Proto、14,595 条 instruction、2,772 个
  constant，覆盖 36/38 opcode；正向 manifest 为
  `tests/compatibility/bytecode-structured-bounded-48.json`；
- `target/compatibility/bytecode-structured-bounded-60-final/report.json` 为 60/60 passed、
  0 difference、0 infrastructure failure，共 596 个 Proto、32,583 条 instruction、5,493 个
  constant，覆盖 36/38 opcode；正向 manifest 为
  `tests/compatibility/bytecode-structured-bounded-60.json`；
- `target/compatibility/bytecode-structured-bounded-72-final/report.json` 为 72/72 passed、
  0 difference、0 infrastructure failure，共 676 个 Proto、40,137 条 instruction、6,669 个
  constant，覆盖 36/38 opcode；正向 manifest 为
  `tests/compatibility/bytecode-structured-bounded-72.json`，并包含全部 24 个 vendored official
  Lua 文件；
- `target/compatibility/m2-compiler-limits-bytecode-131-final/report.json` 为 131/131 passed、
  0 difference、0 infrastructure failure，共 753 个 Proto、42,912 条 instruction、7,427 个
  constant，覆盖 36/38 opcode；正向 manifest
  `tests/compatibility/bytecode-structured-valid-131.json` 是全部 132 个 Lua 文件减去唯一刻意
  非法语法负例的 exact set；
- 131-case corpus 已覆盖固定 C++ 合同中的全部 36 个 frontend-produced opcode。仅缺的
  `NOT/TESTSET` 都有 oracle 的 executable negative contract；当前不再存在未纳入的有效仓库
  源码；显式错误/限制合同也已完成，下一入口不是强造 frontend 不产生的 opcode，而是真实
  VM trace 与剩余 process observable。
  不得把当前通过外推成 M2.2/M2.3 或全部 compiler parity 完成。

真实两端 Snapshot/Basic 与 Diff/Basic 已形成可比较合同，fresh 12-case artifact 位于
`target/compatibility/m2-callback-final-pinned/`，两种 Basic 模式均为 12/12。
同次 Complete/Diff 核心 4-case 为 4/4；独立 Snapshot/Complete 核心也为 4/4，callback
Snapshot/Diff 双 2-case 亦为 2/2。随后 C++ allocator retirement hook 与双 producer 自检
关闭 M2.7 lifecycle；M2.16 的参数/stdin/动作顺序/hash-bang/source-name/文件错误/交互恢复
由双实现 CLI 18/18 关闭，M2.17 的多行/不完整输入、stack reset、history/meta/completion
由双实现 REPL 8/8 关闭；M2.18 viewer 由 15/15 合同关闭。binary chunk execution 仍显式路由 M3。

## 5. 标准库可观察差异

### 5.1 userdata 的 `print` 表示：2 项

| Case | Rust | C++ |
|---|---|---|
| `regressions.move-bug` | `f = userdata: 0x...` | `f = unknown` |
| `functions.call-success` | 两个 file userdata 都显示 `userdata: 0x...` | 都显示 `unknown` |

两个 case 的 `type(f)` 都是 `userdata`，文件打开成功；差异是 base `print`
的值渲染，不是 MOVE、CALL 或 `io.open` 失败。Rust
`append_print_value` 为 userdata/thread/lightuserdata 输出地址；固定 C++
`luaB_print` 只专门处理 string、number、boolean、nil、table、function，
其它类型固定输出 `unknown`。

建议 P1：

- 以 `regressions.move-bug` 为最小 case，先让 `print` 精确复刻固定 C++；
- 分开测试 `print(v)`、`tostring(v)` 和 `__tostring`，不要用一个 helper
  无意改变三套合同；
- 禁止在 golden output 中保留真实 pointer，因为地址既不稳定也不适合作为
  语义标识。

### 5.2 number-to-string 与 error text：`tables.metamethod-arith`

该 case 有两个独立差异：

- Rust `3.3333333333333335`，C++ `3.3333333333333`；
- Rust arithmetic error 缺 source/line，C++ 带文件路径和 line 141。

固定 C++ 的 `src/common/number_conversion.hpp` 使用
`std::to_chars(..., general, 14)`；Rust `print`/`tostring` 当前使用
`f64::to_string()`。建议 P1 建立单一 Lua number formatter，覆盖：

- integer-looking float、`-0`、subnormal、NaN、Inf；
- 14 位边界、科学计数法切换；
- `print`、`tostring`、concat、string format 和 IO write 的调用点。

source/line 部分归入上一节 P0 error provenance。

### 5.3 error envelope 与 chunk name：3 项关闭、1 项开放

四项主分区及最小证据：

| Case | Rust | C++ | 根因 |
|---|---|---|---|
| `basic.syntax-error` | 与 C++ raw stderr/exit 完全一致 | 同左 | **closed**：CLI envelope、chunk-id、CRLF |
| `integration.env-metatable-loader` | stderr 与 C++ raw bytes 完全一致 | 同左 | **closed**：CALL frame provenance；case 只剩 stdout EOL |
| `stdlib.coroutine-errors` | error object 已去掉内部 `@` | 同左 | **closed**：case 只剩 stdout EOL |
| `tables.metamethod-arith` | 无 source/line | `path:141: ...` | **open**：arithmetic error 绕过 location constructor |

建议先定义结构化内部错误：

`{category, object, source_kind, source_bytes, line, traceback}`，

再分别实现 Lua error object、coroutine resume result 和 CLI stderr renderer。
如果直接在各调用点拼字符串，会继续出现顶层多前缀、嵌套少前缀和 `@` 泄露。

验收不能只比较去路径后的尾部 message；必须分别断言 category、error object、
chunk-id、line、CLI program prefix 和 exit code。

### 5.4 GC control 与计账：7 项

受影响：

- `stdlib.collectgarbage-count`
- `stdlib.collectgarbage-simple`
- `stdlib.collectgarbage`
- `stdlib.gcinfo-return`
- `stdlib.gcinfo-simple`
- `stdlib.gcinfo`
- `stdlib.print-gcinfo`

最小 `stdlib.collectgarbage-count`：

- Rust：`136 KB`
- C++：`66.95703125 KB`

综合 `stdlib.collectgarbage` 还暴露：

- Rust `setpause`/`setstepmul` 返回 `0`；
- C++ 返回旧值 `200`；
- Rust before/after/after-GC 为 `40/48/24 KB`；
- C++ 为约 `74.30/74.40/74.15 KB`。

这不是可安全归一化的 allocator noise。上述差异来自旧 baseline artifact；
当前 Rust 已删除 `poll_gcinfo_kb`/`gcinfo_kb` 模拟字段，
`collectgarbage("collect")` 会运行真实 full STW，`gcinfo/count` 读取 collector
accounted bytes。尚未刷新的差异包括：`step` 仍以固定 cycle 决定何时触发
full STW，`setpause`/`setstepmul` 也固定返回 0。固定 C++
`luaB_gcinfo`/`luaB_collectgarbage` 读取 collector 的 total memory，并返回
真实旧参数。

建议 P0，但依赖 M1 的 sweep/root/barrier/shutdown 闭环：

1. `completed-local`：删除 Lua-visible 的模拟 `gcinfo_kb`，让 explicit collect
   执行真实 full STW；
2. collector 已暴露 object count 与 accounted bytes；继续补 allocator
   live/peak bytes；
3. 用 incremental work unit 删除剩余 step 倒计时，并实现
   stop/restart/step/setpause/setstepmul 的真实状态机和旧值返回；
4. 若 Rust/C++ 对象布局不同，使用明确的 Lua logical allocation accounting
   对齐 oracle，不得硬编码某次运行的 `67`；
5. acceptance 同时检查：返回类型、旧参数、step completion、collect 后
   live/object 数下降、shutdown 为零；精确 KB 输出只在 deterministic
   logical-accounting 层比较。

### 5.5 IO text mode 与 side effects：落盘已关闭，剩余 1 项

旧 `stdlib.iolib-core` side effect 是：

| Engine | 文件内容 hex | bytes |
|---|---|---:|
| Rust（旧） | `616c7068610a626574610a` | 11 |
| Rust（fresh） | `616c7068610d0a626574610d0a` | 13 |
| C++ | `616c7068610d0a626574610d0a` | 13 |

该差异现已关闭。Rust file state 保留原始 `b` mode；Windows text file 在逻辑缓冲
与物理落盘边界执行 CRT-compatible 编解码，binary/direct-write 路径保持 raw。
`stdlib.iolib` 的三个文件 side effect 也已逐字节相等：

- `test_output.txt` 两端都是 35 bytes；
- `test_buffer.txt` 两端都是 15 bytes；
- `test_flush.txt` 两端都是 17 bytes。

Lua-visible text `seek/tell` 已关闭：Windows 文本路径文件现在显式维护 logical buffer cursor
与 CRT physical opaque offset 的双向映射，`stdlib.iolib` 的 `f:seek("end")` 已由 32 对齐为
35；binary 与 `tmpfile` offset 合同保持不变。定向回归覆盖 `set/cur/end`、正负 offset 与
CRLF/CRCRLF。

已完成的 byte/process coverage 包括 embedded `\r\n`、裸 `\r`、裸 `\n`、NUL、`0xff`、
text/binary write 和 text read 的 CRLF/Ctrl-Z。后续只把 seek/tell 与 process handle 纳入
该根因，不重新打开已关闭的落盘 EOL。

`io.popen` 也已实现并注册。当前 M2 unrestricted 默认与既有 `os.execute` 一致，Windows 经
`cmd /C`、POSIX 经 `sh -c`；read/write pipe、文本 EOL、成功/非零 close tuple、显式 close 与
Runtime resource drain 都有回归。未来 M4 sandbox capability 必须把 process 默认策略收敛成
配置合同，但不再以 nil function 暴露。

## 6. 最小代表集

修复时不应每次先跑 92 个进程。以下 6 组现有 case 覆盖当前 12 个 status debt；完整
suite 仍是最终门：

| 顺序 | Case | 覆盖 |
|---:|---|---|
| 1 | `integration.env-metatable-loader` | 双方共同 expected-exit failure、loader environment |
| 2 | `functions.call-success` + `regressions.move-bug` | userdata `print` |
| 3 | `tables.metamethod-arith` | number format、arith error provenance |
| 4 | `stdlib.collectgarbage-count` | GC logical bytes |
| 5 | `stdlib.collectgarbage` | GC control state/old values |

还应新增三个更小的 differential case：

- `cli-source-bytes-minimal.lua`：`中文✓`、NUL/高位 byte 的长度与 hex；
- `vm-error-location-minimal.lua`：CALL nil、arith table、metamethod error；
- `io-text-binary-minimal.lua` 已由真实 `lua_app` 子进程测试覆盖同一 byte vector 的
  text/binary physical bytes；若升入 differential manifest，应复用该精确合同。

## 7. 推荐开发顺序

### P0-A：冻结 fresh M1 后基线

执行状态（2026-08-12）：`completed-local`。fresh artifact、runner/executable SHA、
status/normalized/raw/side-effect 计数与 case 移动已记录；正式 status 为 80/12。

1. 当前并行代码稳定后重建 `lua_app`；
2. 不覆盖旧 artifact，输出
   `target/compatibility/m2-compiler-limits-final-m0-pinned/non-official.json`；
3. 记录 Rust/C++ executable SHA-256、工作树状态、runner version；
4. 重算 status 80/12、manifest-normalized 全通道 81/92、direct raw stream/exit 78/92；
5. side-effect observation 现为 92/92 相等；分开报告 11 个 normalization 后 stdout
   差异与 3 个 executable identity raw 差异；
6. 对每个移动的 case 记录 `old -> new` 及导致变化的提交/工作树范围。

### P0-B：compiler/VM 的结构化 parity

1. 当前有效源码 structured bytecode 131/131 已零差异；compiler error/limit matrix 已完成；
2. CALL/RETURN convention 与额外 MOVE 已关闭；
3. 当前 corpus 的 constant/order/max-stack/Proto metadata 与 register/constant 极限合同已对齐；
4. Snapshot/Basic 与 Diff/Basic 真实两端 trace 均已实现且代表集 12/12；Complete 核心
   Snapshot/Diff 双 4-case 与 callback 双 2-case 也已对齐；继续 C++ adapter GC state
   identity/oracle hook 缺口；
5. runtime error provenance 已关闭 CALL/chunk-id/CLI 三项；继续 arithmetic error object。

### P0-C：真实 GC API

在 M1 GC 安全门完成后接入真实计账、step、参数旧值和 collect observable。禁止
用常量修正 7 个 fixture。

### P0-D：缺功能

`io.popen` read/write/close 与生命周期已实现；后续缺功能优先转向 binary chunk、C API 与
native loader，各自仍由 M3 跟踪。

### P1：可观察 stdlib 合同

1. C++ 兼容 number formatter；
2. `print`/`tostring`/`__tostring` 分层；
3. IO text/binary translation、side effects、text seek/tell 与 `popen` 已完成；
4. parser/runtime/coroutine/CLI 各层 error rendering。

### P2：平台观察与 runner 可读性

1. 将已关闭的 EOL 与仍存在的 executable identity 作为诊断标签展示；
2. 保留 raw evidence；只有 manifest 已声明的 identity normalization 可进入 status 判定；
3. 只允许 manifest 明确、窄范围的 normalization；
4. 报告同时展示 raw、declared-normalized 与 semantic-probe 三列。

## 8. 完成条件

该 backlog 的关闭条件不是“L2 后 92/92 相等”，而是：

- fresh artifact 无 runner error、timeout、silent skip；
- 每个 raw difference 要么修复，要么有已批准且有 ID 的 deviation；
- path/EOL normalization 只作用于声明的 channel/case，并保留 raw evidence；
- 11 个当前 status debt 均有最小 regression 或明确 root；
- 当前有效源码 bytecode 131/131 通过且不再有 missing evidence；
- 真实 VM trace 核心 manifest 已通过；更广 callback/GC-reuse 诊断无未解释差异后方可关闭；
- GC case 使用真实计账/状态机，不再使用模拟数字；
- IO side-effect bytes 在 text/binary 两种模式与固定 C++ 同平台一致；
- `io.popen` 不再以 nil function 暴露；
- 最终运行完整 132-item manifest、official strict/slow、bytecode 和 VM trace
  gate。

## 9. 当前 11 项 status debt 主分区核对

| 分区 | Case 数 | Case |
|---|---:|---|
| userdata print | 2 | `functions.call-success`、`regressions.move-bug` |
| number format + error location | 1 | `tables.metamethod-arith` |
| GC | 7 | `stdlib.collectgarbage-count`、`stdlib.collectgarbage-simple`、`stdlib.collectgarbage`、`stdlib.gcinfo-return`、`stdlib.gcinfo-simple`、`stdlib.gcinfo`、`stdlib.print-gcinfo` |
| expected exit | 1 | `integration.env-metatable-loader` |
| **合计** | **11** | 无重复、无遗漏；另有 3 个已登记 executable identity raw 差异但不是 status debt |
