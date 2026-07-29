---
status: provisional
milestone: M2
last_updated: 2026-07-26
evidence_snapshot: target/compatibility/non-official.json
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# M2 non-official 差异聚类与开发 backlog

## 结论

当前可审计结论仍是 **92 个已执行 case 中 4 个 raw match、88 个 raw
difference**。没有 timeout、runner error 或 exit-code 差异。本文的聚类只为
决定开发顺序，不修改 runner 的正式判定：

- 直接在 raw 文本上仅折叠 `CRLF -> LF`，69/88 个差异没有剩余可见差别，
  19/88 个仍不同；
- 先应用 manifest 已声明的 per-engine executable path 替换，再仅为诊断折叠
  `CRLF -> LF`，72/88 个差异在**已比较通道**没有剩余可见差别，16/88 个仍
  不同；
- 上述 69 或 72 个都仍是正式报告中的 `difference`，不是语义通过；
- 4 个 raw match 的 stdout/stderr 都为空，其中只有
  `regressions.return-bug` 还比较了一个文件副作用。因此 4 个 match 也不能
  外推成编译器、VM 或标准库整体通过。

88 项的主根因分区如下。每个 case 只在主分区中计数一次；同一 case 的次级根因
会在详情中保留。

| 主分区 | 数量 | 最小现有代表 | 主要归属 | 建议优先级 |
|---|---:|---|---|---|
| 仅 stdout 的 LF/CRLF | 69 | `basic.print` | host text output / runner observation | P2 |
| executable identity + LF/CRLF | 3 | `runtime.arg-minimal` | CLI fixture contract | P2 |
| source bytes / CLI 编码边界 | 1 | `control-flow.simple-if` | CLI → compiler → Lua string | P0 先复核 |
| userdata 的 `print` 表示 | 2 | `regressions.move-bug` | base + IO userdata | P1 |
| error envelope、source 与 line | 4 | `basic.syntax-error`、`integration.env-metatable-loader` | CLI + compiler/VM + base | P0/P1 |
| GC control 与内存计账 | 7 | `stdlib.collectgarbage-count` | runtime GC + base | P0，依赖 M1 GC |
| IO text mode、副作用与 `popen` | 2 | `stdlib.iolib-core`、`stdlib.iolib` | IO + process capability | P0/P1 |
| **合计** | **88** |  |  |  |

首要判断是：进程 stdout 看起来只差换行，不代表生成的 Proto 相同。独立的
representative bytecode artifact 仍是 2/2 semantic failure；两个 case 分别有
38 和 33 个结构化差异。M2 必须并行推进 process observable 与
bytecode/VM trace，不能用前者的“归一化后相等”关闭后者。

2026-07-26 后续纵切片更新（不改写上述 M0 历史快照）：CALL statement
result convention、contiguous RETURN、constant interning/order 与 max-stack 已修；
Rust bytecode JSON schema v2 也补齐无损 constant、递归 sub-Proto、function/
line/local/upvalue evidence。重新构建后，原两个代表 case 的 opcode、constant
和可比较 metadata 已完全一致，各只剩 2 项：固定 C++ printer 不输出 local
names 导致的 `missing-evidence` 与对应 value difference。扩展
`test_closure_pipeline.lua` 仍达到 500 条真实差异上限，nested compiler parity
没有完成。

## 1. 证据范围与判定口径

### 1.1 使用的快照

主要证据：

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

这两个 non-official artifact 早于当前工作树中的 ByteString、IO scoped-access、
runtime ownership 与 GC/root 迁移。特别是
`control-flow.simple-if` 的 mojibake 很可能已被 M1 字节链路修复，但在新
artifact 产生前仍只能标记为“待复核”，不能提前关闭。

### 1.2 三层比较

本文明确分开三种口径：

1. **L0 raw gate**：直接比较 exit、timeout、stdout bytes、stderr bytes 和已声明
   的 side effects。这是正式状态来源，结果为 4 match / 88 difference。
2. **L1 manifest normalization**：只使用 fixture 已声明的 literal/regex 规则。
   例如 runtime arg case 把各自的 `{{executable_slash}}` 替换成
   `<lua-executable>`。报告仍保留 raw bytes 和 difference。
3. **L2 attribution-only normalization**：在 L0 或 L1 结果上临时把
   `\r\n` 视为 `\n`，仅用于回答“首个差异是否只有 Windows text-mode
   换行”。L2 不写回 artifact，也不改变 case status。

raw 69/19 与 L1+L2 72/16 的三项差额就是：

- `runtime.arg-minimal`
- `runtime.arg-negative`
- `runtime.arg-simple`

三者的 executable path 在 L1 已正确变成 `<lua-executable>`，剩余差异只是
LF/CRLF。直接对 raw `lossy_text` 做 L2 时，两个不同 binary 的真实路径仍然
不同，所以会落入 19 项；对 `normalized_lossy_text` 做 L2 时才落入 72 项。

历史 M0 报告写的是“显式路径 + 分析 EOL 后 75/92 全通道一致”。本节的
4 个 raw match + 72 个 attribution-only candidate 看似是 76/92，但其中
`integration.loadfile-dofile-workflow` 的 `side_effects_compared=false`。
因此：

- 76/92 只表示所有**已比较**通道在该诊断下相等；
- 排除未观察副作用的该 case 后，完整通道证据正是 M0 的 75/92；
- 该 case 必须先补 side-effect capture，不能把“未比较”当成“相等”。

这也解释了两种计数，不是 artifact 漂移；两者都不影响 4/88 的正式 raw 状态。

### 1.3 不能从当前 artifact 推导的结论

- 不能说 69 或 72 个 case 已通过语义验收；
- 不能说 4 个 raw match 覆盖了对应功能，因为它们没有 stdout/stderr
  观察，且大部分没有副作用；
- 不能从 process 输出相同推导 opcode、register allocation、constant order、
  Proto debug metadata 或 VM trace 相同；
- 不能从 7 个 GC 数字 case 推导 weak table、finalizer、resurrection 或
  shutdown 的具体根因；它们只证明 GC API 的可观察值不同；
- 不能从 `stdlib.iolib` 一个 case 推导所有未实现函数。该 case 只直接证明
  `io.popen` 缺失；`string.dump`、native `package.loadlib`、C API 等缺口由
  其他 artifact 和 deviation 记录覆盖。

## 2. EOL 与路径噪声

### 2.1 69 个仅 LF/CRLF 的 case

`basic.print` 是最小证据：

| Engine | stdout hex |
|---|---|
| Rust | `48656c6c6f0a` (`Hello\n`) |
| C++ | `48656c6c6f0d0a` (`Hello\r\n`) |

Rust 的 `crates/lua_stdlib/src/base.rs` 先把 `b'\n'` 放进 `Vec<u8>`，再直接
`stdout().lock().write_all(...)`。固定 C++ 的 `src/lib/baselib.cpp` 调用
`std::fputs("\n", stdout)`；Windows C text stream 把 `\n` 写成 CRLF。这解释
了 69 项里首个不同 byte 的稳定形态，但它只是实现级归因，不是 pass 证明。

69 项按 fixture 域列出如下：

- alien-signals（1）：
  `alien-signals.example`
- basic（3）：
  `basic.arithmetic`、`basic.basic`、`basic.print`
- bytecode（2）：
  `bytecode.debug`、`bytecode.basic`
- control-flow（18）：
  `control-flow.break-counter`、`control-flow.break-for`、
  `control-flow.break-minimal`、`control-flow.break-nested`、
  `control-flow.break-simple`、`control-flow.for-simple`、
  `control-flow.if-basic`、`control-flow.if-comparison`、
  `control-flow.if-else`、`control-flow.if-elseif`、
  `control-flow.if-eq-simple`、`control-flow.if-in-while`、
  `control-flow.if-logic`、`control-flow.if-nested`、
  `control-flow.if-simple`、`control-flow.if-truthy`、
  `control-flow.if-smoke`、`control-flow.simple-if-while`
- functions（9）：
  `functions.factorial`、`functions.closure-simple`、
  `functions.direct-call`、`functions.multiret`、
  `functions.simple-call`、`functions.table-constructor-multiret`、
  `functions.vararg-simple`、`functions.vararg-table`、
  `functions.vararg`
- integration（3）：
  `integration.closure-pipeline`、`integration.coroutine-scheduler`、
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
- stdlib（13）：
  `stdlib.coroutine-basic`、`stdlib.coroutine-closures`、
  `stdlib.coroutine-generators`、`stdlib.coroutine-status`、
  `stdlib.coroutine-values`、`stdlib.debug`、`stdlib.getfenv`、
  `stdlib.io-debug`、`stdlib.io-minimal`、`stdlib.io-type`、
  `stdlib.iolib-simple`、`stdlib.setfenv-detailed`、`stdlib.setfenv`
- steps（3）：
  `steps.basic`、`steps.logic`、`steps.table`
- tables（2）：
  `tables.access`、`tables.zero-key`

建议：

1. 不给全部 case 增加无边界的“忽略空白”规则。
2. 把 stdout/stderr text stream 与 file text mode 分开：
   console EOL 可作为平台观察合同；文件内容必须继续按 bytes 比较。
3. 若项目要求同平台 raw 复刻 C++，在统一 host text-output 层实现 Windows
   CRLF，而不是在每个库函数里散落 `\r\n`。
4. 若项目允许跨平台 EOL deviation，normalization 必须是 manifest 显式声明的
   `crlf-to-lf`，artifact 仍记录 raw mismatch，并额外运行 binary-output
   probe 防止规则吞掉任意 byte 差异。

验收：`basic.print`、一个 `io.write`、一个 stderr case 同时覆盖无换行、单换行、
多换行和 embedded `\r`；raw 与 normalized evidence 都必须保留。

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
`runtime.arg-simple` 此后只剩 LF/CRLF。

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
也已接收 byte source，`GcString` 已迁移到 canonical `ByteString`。因此正确
动作不是基于旧 artifact 继续改代码，而是 fresh rerun：

1. 新增或复用最小 probe：源码只包含
   `print("中文✓")`，同时输出 `#s` 和每个 `string.byte`；
2. 在 CLI 文件、stdin、`load` reader、`loadfile` 四个入口运行；
3. 比较 raw stdout hex，不使用 Unicode text normalization；
4. 若新 artifact 已相等，把本 case 标成 M1 修复后的 regression verification；
   若仍不同，再沿 source bytes → lexer string literal → constant →
   `GcString` → `print` 定位首个变化点。

优先级 P0 的含义是“先复核”，不是基于过期证据重写实现。

## 4. 编译器与 VM

### 4.1 process case 暴露的真实问题：error provenance

进程 corpus 中大多数控制流、closure、multiret 和 regression case 在 L2
观察下输出相同，但这只证明最终脚本输出相同。当前明确暴露 VM 问题的是：

- `integration.env-metatable-loader`
  - Rust：`attempt to call global 'tostring' (a nil value)`
  - C++：`lua_app.exe: (load):2: attempt to call global 'tostring' (a nil value)`
- `tables.metamethod-arith`
  - Rust error object 没有 source/line；
  - C++ 包含
    `.../test_metamethod_arith.lua:141: attempt to perform arithmetic ...`。

Rust 的 `crates/lua_vm/src/execute.rs` 已有 `runtime_error_at(proto, pc, ...)`，
但 CALL 非函数、算术/一元错误等分支仍直接构造 `RuntimeError::new(...)`。
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
| `test_bytecode.lua` | failed | 38 |
| `test_bytecode_debug.lua` | failed | 33 |

以 `test_bytecode.lua` 为例，C++/Rust 仍有：

- constants 长度 3 vs 4，Rust tool 没有提供 string constant value；
- instruction 长度 12 vs 13；
- CALL 的 `C` 为 1 vs 2；
- return 前 Rust 多一个 MOVE；
- max stack 6 vs 8；
- Rust 缺 child count、line-defined、upvalue/local-name 等证据。

因此 `bytecode.basic` 虽在 process report 中只差 EOL，也不能进入
compiler-parity completed。建议顺序：

1. P0 先补齐 Rust bytecode JSON 的 constant value、nested Proto、
   line/local/upvalue metadata，消除 `missing-evidence`；
2. 对最小 `print`/return case 对齐 CALL/RETURN result convention；
3. 再处理 constant interning/order、register allocation 和 max stack；
4. 每次变更同时跑 process behavior、structured bytecode 和真实 VM trace。

当前执行结果：

- 第 1–3 项的“原两个简单 case”已完成：两例指令、常量、max stack 与 Rust
  可提供 metadata 一致；
- `target/compatibility/bytecode-schema-v2-original-two/report.json` 中两例
  均只剩 C++ local-name printer 证据缺口；
- `target/compatibility/bytecode-closure-schema-v2/report.json` 证明 nested
  Proto 仍有大量真实差异，下一次应从首个 Proto/PC 分歧开始修；
- 不得把 C++ `localNames=false` 登记成通过，也不得把 500 条截断列表当作
  500 个独立根因。

真实两端 `--trace-diff` 尚未形成可比较合同；现有 VM trace 只有 synthetic
self-test。M2.7/M2.16 在真实 trace 可运行前不能关闭。

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

### 5.3 error envelope 与 chunk name：4 项

四项主分区及最小证据：

| Case | Rust | C++ | 根因 |
|---|---|---|---|
| `basic.syntax-error` | `@path:7: ...` | `lua_app.exe: path:7: ...` | CLI envelope；`@` 未按 chunk-id 规则去除 |
| `integration.env-metatable-loader` | 无 source/line/progname | `lua_app.exe: (load):2: ...` | CALL error 丢 frame provenance |
| `stdlib.coroutine-errors` | error object 中为 `@path:line` | `path:line` | coroutine 传播保留了内部 `@` |
| `tables.metamethod-arith` | 无 source/line | `path:141: ...` | arithmetic error 绕过 location constructor |

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

### 5.5 IO text mode 与 side effects：2 项

`stdlib.iolib-core` 的 stdout 在 L2 相等，但 side effect 仍不同：

| Engine | 文件内容 hex | bytes |
|---|---|---:|
| Rust | `616c7068610a626574610a` | 11 |
| C++ | `616c7068610d0a626574610d0a` | 13 |

这不是“只影响控制台”的噪声。Rust `std::fs::File::write_all` 按 bytes 写入；
固定 C++ 通过 `_fsopen`/`fopen` 的 text mode 写入，Windows CRT 做 LF→CRLF。
`stdlib.iolib` 同样看到：

- `test_output.txt` 32 vs 35 bytes；
- `test_buffer.txt` 14 vs 15 bytes；
- seek-end 报告 32 vs 35。

建议 P1：

1. IO handle 保留 text/binary mode，不要仅在 open 时删掉 `b`；
2. Windows text write 实现 LF→CRLF，text read 实现对应折叠，binary mode
   必须逐 byte 原样；
3. seek/tell、append、buffer flush 与 side-effect snapshot 按 C++ 同平台
   规则验证；
4. corpus 加 embedded `\r\n`、裸 `\r`、裸 `\n`、NUL、`0xff`，防止双重转换。

`stdlib.iolib` 还直接证明缺失功能：

- Rust：`attempt to call field 'popen' (a nil value)`
- C++：`Pipe opened successfully`，随后输出 `Hello from pipe`

固定 C++ `src/lib/iolib.cpp` 在 process capability 允许时注册 `io_popen`，
Windows 使用 `_popen`；Rust 当前没有 `popen` 实现或注册。该 case 应按
**缺功能** 处理，不得归入错误文本或 EOL。

建议 P0/P1：

- 先确定与 C++ 相同的 process capability 默认策略；
- 实现 read/write mode、close status、command failure 和 handle lifecycle；
- 用无 shell quoting 歧义的固定命令做 Windows/Linux 双平台测试；
- 如果安全策略决定不支持，必须形成显式批准 deviation；在此之前状态是 open，
  不能把 nil-function error 当兼容行为。

## 6. 最小代表集

修复时不应每次先跑 92 个进程。以下 11 个现有 case 能覆盖当前所有根因；完整
suite 仍是最终门：

| 顺序 | Case | 覆盖 |
|---:|---|---|
| 1 | `basic.print` | stdout LF/CRLF |
| 2 | `runtime.arg-minimal` | executable normalization + arg index |
| 3 | `control-flow.simple-if` | UTF-8 source/string/output bytes |
| 4 | `basic.syntax-error` | parser error、chunk id、CLI prefix |
| 5 | `integration.env-metatable-loader` | load chunk、CALL error source/line |
| 6 | `regressions.move-bug` | file userdata `print` |
| 7 | `tables.metamethod-arith` | number format、arith error provenance |
| 8 | `stdlib.coroutine-errors` | coroutine error object 与 `@` stripping |
| 9 | `stdlib.collectgarbage-count` | GC logical bytes |
| 10 | `stdlib.collectgarbage` | GC control state/old values |
| 11 | `stdlib.iolib-core` + `stdlib.iolib` | text/binary side effect、seek、`popen` |

还应新增三个更小的 differential case：

- `cli-source-bytes-minimal.lua`：`中文✓`、NUL/高位 byte 的长度与 hex；
- `vm-error-location-minimal.lua`：CALL nil、arith table、metamethod error；
- `io-text-binary-minimal.lua`：同一 byte vector 分别以 text/binary mode
  round-trip。

## 7. 推荐开发顺序

### P0-A：冻结 fresh M1 后基线

1. 当前并行代码稳定后重建 `lua_app`；
2. 不覆盖旧 artifact，输出
   `target/compatibility/non-official-m1-current.json`；
3. 记录 Rust/C++ executable SHA-256、工作树状态、runner version；
4. 重算 raw 4/88、raw+EOL 69/19、declared-normalization+EOL 72/16；
5. 为 `integration.loadfile-dofile-workflow` 补齐 side-effect observation，
   分开报告 compared-channel 76/92 与 complete-channel 75/92；
6. 对每个移动的 case 记录 `old -> new` 及导致变化的提交/工作树范围。

### P0-B：compiler/VM 的结构化 parity

1. 补 bytecode tool 缺失证据；
2. 修 CALL/RETURN convention 与额外 MOVE；
3. 对齐 constant/order/max-stack/Proto metadata；
4. 实现真实两端 trace，再定位首个 VM state divergence；
5. 统一 runtime error provenance。

### P0-C：真实 GC API

在 M1 GC 安全门完成后接入真实计账、step、参数旧值和 collect observable。禁止
用常量修正 7 个 fixture。

### P0-D：缺功能

实现 `io.popen` 及生命周期，或建立明确批准的 deviation。当前目标是完整复刻
lua_cpp，因此默认路线是实现。

### P1：可观察 stdlib 合同

1. C++ 兼容 number formatter；
2. `print`/`tostring`/`__tostring` 分层；
3. IO text/binary translation、seek 和 side effects；
4. parser/runtime/coroutine/CLI 各层 error rendering。

### P2：平台观察与 runner 可读性

1. 将 EOL 与 executable identity 作为诊断标签展示；
2. 保留 raw failure count，不把标签改写成 pass；
3. 只允许 manifest 明确、窄范围的 normalization；
4. 报告同时展示 raw、declared-normalized 与 semantic-probe 三列。

## 8. 完成条件

该 backlog 的关闭条件不是“L2 后 92/92 相等”，而是：

- fresh artifact 无 runner error、timeout、silent skip；
- 每个 raw difference 要么修复，要么有已批准且有 ID 的 deviation；
- path/EOL normalization 只作用于声明的 channel/case，并保留 raw evidence；
- 16 个当前非噪声 case 均有最小 regression；
- representative bytecode 2/2 通过且不再有 missing evidence；
- 真实 VM trace 支持并在代表 corpus 无未解释差异；
- GC case 使用真实计账/状态机，不再使用模拟数字；
- IO side-effect bytes 在 text/binary 两种模式与固定 C++ 同平台一致；
- `io.popen` 不再以 nil function 暴露；
- 最终运行完整 131-item manifest、official strict/slow、bytecode 和 VM trace
  gate。

## 9. 88 项主分区核对

| 分区 | Case 数 | Case |
|---|---:|---|
| 仅 EOL | 69 | 见 2.1 的完整域列表 |
| executable + EOL | 3 | `runtime.arg-minimal`、`runtime.arg-negative`、`runtime.arg-simple` |
| source bytes | 1 | `control-flow.simple-if` |
| userdata print | 2 | `functions.call-success`、`regressions.move-bug` |
| error/source/line | 4 | `basic.syntax-error`、`integration.env-metatable-loader`、`stdlib.coroutine-errors`、`tables.metamethod-arith` |
| GC | 7 | `stdlib.collectgarbage-count`、`stdlib.collectgarbage-simple`、`stdlib.collectgarbage`、`stdlib.gcinfo-return`、`stdlib.gcinfo-simple`、`stdlib.gcinfo`、`stdlib.print-gcinfo` |
| IO | 2 | `stdlib.iolib-core`、`stdlib.iolib` |
| **合计** | **88** | 无重复、无遗漏 |
