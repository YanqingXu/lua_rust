---
title: lua_rust 完整复刻 lua_cpp 开发计划
status: in-progress
current_milestone: M1
m0_status: completed
last_updated: 2026-07-29
rust_baseline: 6284135
cpp_oracle: 87c15e6
lua_oracle: Lua 5.1.5
---

# lua_rust 完整复刻 lua_cpp 开发计划

## 1. 目标与范围

`lua_rust` 的目标是以 Rust 完整复刻 `lua_cpp` 的可观察逻辑和对外能力。
`lua_cpp` 是项目特有行为的真值来源；Lua 5.1.5 官方实现是标准语言行为的第二真值来源。

“完整复刻”不要求逐行翻译 C++，但要求以下合同全部成立：

1. Lua 源码在两个实现中的返回值、输出、错误、退出状态和副作用一致。
2. 编译器生成的 opcode、操作数、常量、子 Proto 和调试信息一致。
3. VM 的调用帧、upvalue、vararg、多返回、coroutine、metamethod 和错误传播一致。
4. GC 的回收、弱表、finalizer、resurrection、write barrier 和关闭语义一致。
5. Lua 5.1 C API、辅助库、标准库、二进制块和动态模块合同一致。
6. `lua_cpp` 已提供的 sandbox、资源预算、取消、观测和 SDK 能力在 Rust 中有等价实现。
7. 所有保留差异都必须显式登记、获得批准并由测试锁定。

本计划优先保证正确性和可验证性。在行为对齐完成前，不以性能优化、内部重构或增加
非 `lua_cpp` 功能作为主要目标。

## 2. 当前基线

### 2.1 版本基线

| 项目 | 当前基线 | 用途 |
|---|---|---|
| `lua_rust` | `6284135` | 本计划起点 |
| `lua_cpp` | `87c15e6` | 项目行为 oracle |
| 官方 Lua | Lua 5.1.5 | 标准行为 oracle |

`lua_cpp` 基线升级必须通过独立 PR 完成。升级 PR 必须列出新增/变化合同、差分结果和
Rust 侧新增任务，禁止在普通功能 PR 中静默移动 oracle。

### 2.2 已有能力

- 6 crate Cargo workspace 已建立。
- Lexer、Parser、AST、CodeGen 和 38 条 opcode 主链已打通。
- VM 已覆盖函数、闭包、upvalue、vararg、多返回、循环、表、主要 metamethod 和 coroutine。
- base、math、string、table、io、os、coroutine、debug、package 已有较宽的函数表面。
- M1.8 shutdown、P1 collector provenance、managed Proto、publication-root
  基础、`TEMPORARY_STATE_ROOTS/PendingState` 及 Runtime coroutine activation
  trampoline、compiler Proto→Function、library registration 与 package
  publication 切片完成后，fmt、all-targets Clippy、warning-free rustdoc、
  `cargo check --workspace --all-targets` 和 758 个
  全 workspace Debug tests 已再次通过；远程 CI 不再允许隐式跳过
  security audit。
- fixture manifest 当前共 131 项：101 个 non-official、24 个 official 和
  6 个 differential（4 个 M0 focused cases，加 2 个 M1 raw-byte cases）。
- 101 个 non-official 中执行 92 个、跳过 9 个 helper；92/92 退出码一致，
  raw 全通道匹配 4/92、差异 88、runner error 0、timeout 0。显式路径加分析
  EOL 规范化后全通道一致 75/92，但 raw 差异仍完整保留。
- 4 个 focused differential case 在官方 Lua 5.1.5 和
  `lua_cpp@87c15e6` 两条 lane 上均为 4/4；stock `_VERSION` 差异显式引用
  `NOTE-001`。
- 本地统一 M0 gate 为 `passed=true`、`hardFailures=0`、3 项债务，
  用时 11,765 ms；远程 CI 尚待首次运行。

### 2.3 已确认的主要阻塞

| 领域 | 当前问题 | 优先级 |
|---|---|---|
| 验证 | M0 runner 已建立且 fail-closed；当前 non-official 仍有 88 个语义差异 | P0 |
| CI | 本地统一质量/M0 门已通过，远程 workflow 尚待首次运行 | P1 |
| 字符串 | ByteString/GcString/StringPool、编译器和主要宿主边界已迁移 arbitrary bytes；binary chunk、C API 与“所有生产字符串必须 intern”的强制合同仍待后续门槛 | P0 |
| GC | VM 的 `collectgarbage` 路径不 sweep；`gcinfo/step` 是模拟计数 | P0 |
| 生命周期 | Runtime/StateArena 已有确定性 state→Thread→ordinary→fixed 销毁与 1000 轮归零测试；关闭期 Lua `__gc`、显式服务 drain、Heap/allocator owner 仍未闭环 | P0 |
| GC 引用安全 | `GcRef` 已携带进程级不复用 `ObjectId`，collector live table 在解引用前校验地址、身份与类型；managed Proto、checked open-Upvalue owner、临时对象根与 `PendingState` 状态根已落地。`Value::String` 内容 Eq/Hash 和未迁移 production publication 路径仍阻止 live sweep | P0 |
| GC 安全 | table/function/upvalue 等写入路径尚未接入 write barrier | P0 |
| IO | raw stdin/stdout/stderr、文件/tmpfile 与 scoped userdata access 已通过本地 byte/process 测试；text mode、popen 与显式 shutdown 服务语义仍待 M2/M1.12 | P1 |
| Bytecode parity | schema v2 已补齐 constant/sub-Proto/function/line/upvalue 证据；原两例 opcode/constant/metadata 已完全一致，各只剩 2 项由固定 C++ printer 不输出 local names 导致的 fail-closed 证据差异。扩展 closure case 仍触发 500 条真实差异上限 | P0 |
| VM trace parity | runner 自检通过，但两端真实 `--trace-diff` 支持仍缺失 | P0 |
| C API | 无 `lua_capi`、公开头、staticlib/cdylib 和 123 项官方 API 合同 | P1 |
| 二进制块 | 旧进程内 registry 已删除；`string.dump` 在真实 serializer 前显式返回 unsupported，尚无可持久化 binary chunk | P1 |
| 动态模块 | `package.loadlib` 明确不支持动态库 | P1 |
| 生产能力 | sandbox、预算、取消、owner-thread、metrics、worker 尚未迁移 | P2 |
| 文档 | M0/Phase 1–5 报告与 11 项 deviation 已建立；后续随证据持续更新 | P1 |

## 3. 执行原则

### 3.1 双 oracle

- Lua 5.1 标准行为：同时比较官方 Lua 5.1.5、`lua_cpp` 和 `lua_rust`。
- `lua_cpp` 扩展行为：以固定 SHA 的 `lua_cpp` 为准。
- 两个 oracle 不一致时，先建立最小复现，再由维护者决定目标行为。
- 决定结果必须进入 `docs/rust_migration/deviation_log.md`。

### 3.2 验证先于功能

每项迁移必须先具备失败测试或差分用例。不得仅凭函数名、代码结构或内部单元测试宣布完成。

### 3.3 字节模型先于 C API

在 Lua 字符串仍由不一致的 UTF-8/Latin-1 shim 表示时，不开始正式 C API、binary chunk
和动态模块工作，避免围绕错误 ABI 返工。

### 3.4 所有权先于 sweep

在 coroutine state、dump Proto、VM root set 和 shutdown 所有权闭环完成前，不把真实
sweep 直接接入现有 VM。否则当前的内存泄漏可能变成悬垂指针和 use-after-free。

### 3.5 禁止静默近似

占位实现、固定返回值、模拟计数和 unsupported 分支必须满足至少一项：

- 转换为显式 `NotImplemented`/feature gate；
- 进入 deviation log；
- 有明确任务 ID 和阻塞测试；
- 在兼容报告中标为 expected failure。

不得用只验证同一个模拟实现的测试作为兼容完成证据。

## 4. 总体里程碑

```text
M0 可信基线与验证闭环
  ↓
M1 字节字符串、所有权与真实 GC
  ↓
M2 Lua 源码、标准库与工具行为对齐
  ↓
M3 Binary Chunk、C API 与动态模块
  ↓
M4 生产运行时、安全与观测
  ↓
M5 跨平台 SDK、性能与发布
```

M0 是所有工作的共同前置。M1 完成后，M2 的源码兼容和 M3 的 C API 边界可以有限并行；
M4 必须建立在 M1 和 M3 的生命周期、allocator 和公开状态合同之上。

### 4.1 里程碑执行状态

| 里程碑 | 状态 | 当前证据或入口 |
|---|---|---|
| M0 | `completed` | 本地统一 M0 gate 通过，0 hard failure、3 项已登记债务；详见 [M0 收口报告](docs/rust_migration/m0_report.md)。 |
| M1 | `active` | P1 provenance、managed Proto、shutdown、临时对象/状态根、StateHandle fail-closed identity/generation、Runtime coroutine activation trampoline、checked open-Upvalue owner、compiler Proto→Function 与 library/package publication 切片已完成；IO construction graph 是下一条 publication 主线。 |
| M2 | `active-limited` | 原两例 bytecode 指令/常量/metadata 已对齐；C++ local-name 证据缺口、nested Proto 大量差异、88 个 non-official 差异和真实 VM trace 仍开放。 |
| M3 | `pending` | 等待 M1 的字节表示和生命周期合同稳定。 |
| M4 | `pending` | 等待 M1/M3 的 runtime、allocator 与公开状态合同。 |
| M5 | `pending` | 等待兼容性与生产运行时门槛。 |

这里的 M0 `completed` 表示本地验证基础设施和 fail-closed 报告合同达到 M0
完成门槛，不表示 3 项语义债务已经修复，也不表示总计划完成。远程 CI 尚待首次
运行；其结果若暴露 workflow 或平台故障，必须重新打开对应 M0 项。

## 5. M0：可信基线与验证闭环

### 5.1 M0 目标

建立可以持续回答“Rust 与 C++ 还差什么”的自动化系统，修复当前 CI 自身的确定性故障。

### 5.2 任务清单

#### M0.1 固定 oracle

- 新建机器可读的 oracle 配置，建议为 `tests/compatibility/oracle.toml`。
- 记录：
  - C++ 仓库 URL 和 commit SHA；
  - 官方 Lua 5.1.5 来源及 SHA-256；
  - official suite 文件清单和 SHA-256；
  - 差分输出 schema 版本。
- CI 使用第二次 checkout 将 `lua_cpp` 固定到指定 SHA，禁止使用浮动 `main`。
- oracle 更新必须生成差分摘要。

验收：

- 任意 CI run 都能准确报告所用 C++ SHA 和 Lua oracle hash。
- 修改 oracle SHA 会触发专门的 baseline-change 检查。

#### M0.2 修复基础质量门

- 删除 `.cargo/config.toml` 中递归的 `nextest` alias，或改名为 `nt`。
- 在 nextest 修复前，以 `cargo test --workspace` 作为可靠 fallback。
- Clippy 改为：

  ```powershell
  cargo clippy --workspace --all-targets -- -D warnings
  ```

- 清理当前 104 条 test-target Clippy 诊断。
- 文档检查增加：

  ```powershell
  $env:RUSTDOCFLAGS = "-D warnings"
  cargo doc --workspace --no-deps
  ```

- 增加 Release 测试。
- 避免 workflow 逐项运行后再重复执行完整质量脚本。
- 区分 `cargo audit` 工具缺失、网络失败和真实漏洞。

验收：

- 本地质量脚本和 CI 使用同一入口。
- Debug/Release、fmt、all-targets clippy、doc 全部通过。
- CI 中不存在被标记为成功但实际未执行的步骤。

#### M0.3 Lua fixture manifest

- 为 `tests/lua` 建立 manifest。
- 每个文件至少登记：
  - `id`；
  - 相对路径；
  - 类型：`entry`、`helper`、`negative`、`manual-output`；
  - 所需工作目录；
  - 所需参数和环境；
  - 预期退出状态；
  - timeout；
  - 是否比较 stdout/stderr；
  - 是否允许非确定字段；
  - 适用 oracle。
- 不直接逐文件执行 helper/module 文件。
- 将 87 个无 assert 的脚本逐步转换为：
  - 明确 assert；
  - golden 输出；
  - 结构化差分。

验收：

- 125 个现有脚本全部进入 manifest，不能存在未分类文件。
- runner 能正确区分预期语法错误和真实失败。

#### M0.4 进程级 CLI runner

- 为 `lua_app` 建立真正的子进程测试。
- 捕获并比较：
  - stdout bytes；
  - stderr bytes；
  - exit code；
  - timeout；
  - 输出文件和其他显式副作用。
- 支持路径、地址、临时目录等非确定字段的受控规范化。
- 禁止无规则地忽略整行或整段错误。
- 首先覆盖：
  - basic；
  - control_flow；
  - functions；
  - regressions；
  - runtime；
  - stdlib；
  - tables。

验收：

- 当前 101 个非 official 脚本的结果形成机器可读 JSON artifact。
- 每个差异都有 case ID、C++ 结果、Rust 结果和 diff。

#### M0.5 恢复标准差分

- 从 C++ 迁入 4 个 differential cases：
  - `value-types`；
  - `error-category`；
  - `gc-weak-value`；
  - `stderr`。
- 建立两条 lane：
  - Rust vs 官方 Lua 5.1.5；
  - Rust vs 固定 SHA 的 `lua_cpp`。
- 先修复宿主 stdin/stdout/stderr，使 runner 能真实观察结果。
- 将 `_VERSION`、错误类别、弱表可观察副作用纳入 schema。

验收：

- 4 个用例能够被执行和比较，不允许因空输出、skip 或 runner 错误假绿。
- 第一阶段允许语义失败，但基础设施必须稳定生成报告。
- M2 完成条件要求 4/4 通过或存在已批准 deviation。

#### M0.6 恢复 bytecode 与 VM trace 差分

- 从 `012b449^` 恢复并更新：
  - `tools/compare_bytecode.ps1`；
  - `tools/compare_vm_trace.ps1`。
- bytecode 比较至少包括：
  - 38 opcode 序列；
  - 32-bit instruction word；
  - A/B/C/Bx/sBx；
  - RK operand；
  - 常量值及顺序；
  - 子 Proto；
  - 参数、vararg、max stack；
  - line info、local、upvalue names。
- VM trace 比较至少包括：
  - pc/opcode；
  - 活跃调用帧；
  - stack top；
  - changed registers；
  - upvalue open/close；
  - call/return/yield/resume；
  - 错误值和错误类别。

验收：

- 每次失败都保存最小输入和结构化 diff。
- PR 快速门运行代表性语料；nightly 运行完整语料。

#### M0.7 文档和状态治理

- 将 README 的 Phase 1–5 状态改为证据驱动。
- 建立：
  - `phase_1_report.md`；
  - `phase_2_report.md`；
  - `phase_3_report.md`；
  - `phase_4_report.md`；
  - `phase_5_report.md`。
- 将当前已知近似全部写入 deviation log：
  - 模拟 GC；
  - 非标准 dump；
  - memory stdio；
  - dynamic library unsupported；
  - OS/locale/time 近似；
  - Unicode/byte-string 差异；
  - C API 缺失。

验收：

- README、phase report、代码和 CI 之间不存在已知状态矛盾。
- 每个 expected failure 都能反向查到 deviation 或任务 ID。

### 5.3 M0 完成门槛

- 质量门入口稳定通过。
- 125 个 Lua 文件全部有 manifest。
- 进程级 CLI、4-case differential、bytecode 和 VM trace runner 均能生成可靠报告。
- oracle 被固定且可审计。
- 所有当前已知近似进入 deviation log。

### 5.4 M0 执行台账（2026-07-26）

| 任务 | 状态 | 验证证据 |
|---|---|---|
| M0.1 固定 oracle | `completed` | `oracle.toml`、Lua/official source manifest、SHA 校验和 baseline-change policy 均已接入；C++ 固定为 `87c15e6`。 |
| M0.2 基础质量门 | `completed` | 本地 `rust_quality_gate.ps1 -SkipAudit` 通过 fmt、all-targets Clippy、596 个 Debug/Release workspace tests 和 `-D warnings` rustdoc；本地未运行 audit，远程 CI 尚待首次运行。 |
| M0.3 fixture manifest | `completed` | 当前 131 项全部分类：101 non-official、24 official、6 differential；其中 125 项为原有基线，另有 4 个 M0 focused 与 2 个 M1 raw-byte differential。 |
| M0.4 进程级 CLI runner | `completed` | 101 个 non-official 中执行 92、跳过 9 个 helper；92/92 exit 一致，raw match 4、difference 88、error 0、timeout 0；显式路径加分析 EOL 规范化后全通道一致 75/92。 |
| M0.5 标准差分 | `completed` | `value-types`、`error-category`、`gc-weak-value`、`stderr` 在官方 Lua 和固定 C++ 两条 lane 上均 4/4 通过；官方 `_VERSION` 差异由 NOTE-001 批准。 |
| M0.6 bytecode/VM trace 差分 | `completed`（基础设施） | 两个 parity 工具已恢复且 fail-closed 自检通过；真实 bytecode 代表语料仍 2/2 有差异，真实 VM trace `--trace-diff` unsupported，均保留为开发债务。 |
| M0.7 文档与状态治理 | `completed` | README、M0/Phase 1–5 报告与 NOTE-001–010 已形成证据链；NOTE-010 记录进入 M1 后的 hash 决策。 |

统一 M0 gate 的本地结果为 `passed=true`、`hardFailures=0`、3 项债务，
`durationMs=11765`。3 项债务是 non-official 88 个语义差异、代表性
bytecode parity 差异和真实 VM trace unsupported；它们不会把报告误判为基础
设施失败，但必须在 M1/M2 后续工作中消除。详细数据和验证边界见
[M0 收口报告](docs/rust_migration/m0_report.md)。

## 6. M1：字节字符串、所有权与真实 GC

### 6.1 M1 目标

建立可支撑真实回收、binary chunk 和 C API 的安全运行时基础。

### 6.2 字节字符串迁移

#### M1.1 ByteString 设计

- 编写短 RFC，明确：
  - Lua 字符串是任意字节；
  - 内部不可假设 UTF-8；
  - NUL 合法；
  - hash、equality、length 全部按 bytes；
  - CLI 显示与错误显示才进行 UTF-8/lossy 转换；
  - 文件路径和 Lua 字符串是不同类型。
- 推荐表示：

  ```rust
  Box<[u8]>
  ```

  或具有同等不可变、稳定地址和精确长度语义的容器。

#### M1.2 迁移 GcString 与 StringPool

- `GcString::data()` 返回字节切片。
- `as_ptr + len` 指向同一真实缓冲。
- StringPool 使用字节 key。
- hash 与 `lua_cpp`/Lua 5.1 对齐。
- 删除 Latin-1/UTF-8 多套转换 shim。
- 增加：
  - NUL；
  - 0x00–0xFF；
  - 无效 UTF-8；
  - 中文 UTF-8 原始字节；
  - 长字符串；
  - interning identity 测试。

#### M1.3 迁移编译器和宿主边界

- Lexer/Parser 输入改为 bytes 或明确的 byte cursor。
- Lua 语法 token 可按 ASCII 识别，字符串字面量内容必须原样保留。
- `load`、`loadfile`、`loadstring`、CLI stdin/file 都使用统一字节入口。
- string/io/package/os/dump/bytecode 工具统一使用 ByteString。
- 所有文本展示集中到单独 formatting 层。

验收：

- 任意字节字符串可以创建、比较、拼接、查找、dump/load 和经 C API 读取。
- UTF-8 脚本不再双重编码。
- `#string.char(255)`、hash、pointer+length 同时正确。

### 6.3 运行时所有权

#### M1.4 Runtime/EngineContext 所有者

- 引入唯一运行时所有者，统一持有：
  - GarbageCollector；
  - StringPool；
  - main LuaState；
  - coroutine state arena；
  - registry/global roots；
  - allocator；
  - IO/services；
  - 后续 execution policy。
- 禁止由多个局部变量通过裸指针拼装隐式生命周期。
- 明确 State、Thread、Proto、Function 和 collector 的销毁顺序。

#### M1.5 Coroutine state 所有权

- 移除 `Box::into_raw(LuaState)` 无释放路径。
- 优先考虑由 Runtime 管理的 generational arena：
  - `Thread` 只保存稳定 handle；
  - Runtime 负责查找和销毁 coroutine state；
  - handle 失效可检测；
  - 不产生 `&'static mut LuaState`。
- 若采用其他设计，必须证明：
  - 单一所有者；
  - 无 mutable alias；
  - Thread 回收时 State 一定释放；
  - debug/coroutine API 不返回超出作用域的引用。

当前 trampoline 细化执行顺序：

1. **已完成本地基座：** 固定 C++/stock 的 `Normal` 祖先重入
   characterization；实现 `Runtime::drive_state_turns`，保证每个 turn 只借
   一个 state、`Switch` 前释放借用、panic 后恢复 slot 与 active count。
2. **已完成生产切片：** 引入 scoped native mailbox/capability 与独立
   `VmExit::NativeRequest`，只让 `coroutine.resume`/`wrap_runner` 发布
   `ResumeRequest`，禁止在 callback 内递归解析/执行 target。生产入口使用
   sealed context/窄能力，不把可 `mem::take` 或转存 raw pointer 的完整
   `&mut LuaState/GC/StringPool` 暴露给 runtime-native callback。
3. **已完成：** C 调用帧进入 deferred 状态并保存 nonce、`func_pos`、wanted result、
   caller CI/PC；Runtime driver 在 caller guard Drop 后再验证和借 target。
4. **已完成：** Runtime 维护 activation frame 栈而非“唯一 active handle 集合”，允许固定
   C++ 的 `A→B→A` `Normal` 祖先激活环；`Running`/`Dead` 仍拒绝。
5. **已完成本地根种子：** transfer args/results/error、Thread handle 与
   deferred frame 进入 Runtime-owned activation buffer，并由 canonical
   root tracer 扫描；live destructive sweep 仍因其他 root/owner 缺口保持禁用。
6. **已完成当前 envelope：** 对齐 caller `Running↔Normal`、
   callee `Running/Suspended/Dead`、
   caller link、`allow_yield`、saved execution count、resume/wrap envelope、
   精确 0/1/多返回与 error identity/traceback。
7. **已完成生产入口迁移：** app、stdlib harness 和 coroutine
   resume/wrap 已由 Runtime 调度；普通生产执行不再通过长生命周期
   `RuntimePartsMut` 绕过调度器。debug 跨 state API 仍是后续项。
8. **部分完成：** 验收包含普通 A→B→C、`Normal` 祖先重入顺序、yield/resume args、
   pcall/C boundary、wrap error、panic/fault cleanup、临时根、数千层链与
   peak borrowed slots 始终为 1；补充“首 turn 成功后 Switch 到
   foreign/stale/borrowed target”及 turn 内 coroutine create 触发 arena
   slot 扩容的回归。当前已锁定精确 `Normal` 祖先输出、protected `pcall`
   resume、wrap yield/error、stdlib coroutine fixtures、root inventory 与
   workspace 回归；数千层链、广义 fault injection 和 debug 跨 state 矩阵仍待补。

#### M1.6 移除悬垂 registry

- 删除 `dump.rs` 中 thread-local `GcRef<Proto>` registry。
- 在真实 serializer 完成前，非标准 dump 必须显式 feature-gate 或返回 unsupported。
- 所有长期缓存必须：
  - 被 GC root；
  - 或保存可验证的 generational handle；
  - 或保存自有序列化数据。

### 6.4 GC 闭环

#### M1.7 Root inventory

- 建立所有 GC root 的机器可读/文档化清单：
  - global/registry；
  - main/coroutine stacks；
  - CallInfo；
  - open upvalues；
  - active function/proto；
  - C closure upvalues；
  - debug hook/error；
  - pending finalizers；
  - temporary protected roots；
  - library-owned live handles。
- 每种 root 都必须有回归测试。

#### M1.8 确定性 shutdown

- Runtime/collector Drop 必须释放所有普通对象和 fixed 对象。
- shutdown 顺序对齐 `lua_cpp`：
  - 拒绝 busy/foreign-thread close；
  - 运行剩余 `__gc`；
  - 清理 native/module/IO resources；
  - 释放 main/coroutine state；
  - 释放 fixed strings/registry；
  - allocator live bytes 归零。
- CLI 和测试不得依赖进程退出替代析构。

#### M1.9 真实 full collection

- `collectgarbage("collect")` 调用真正的 mark、propagate、weak、finalize、sweep。
- `gcinfo` 来自实际内存计账。
- 普通不可达对象必须被销毁。
- 删除当前 `+8/+24` 和固定 step 次数模拟。

#### M1.10 Incremental GC

- 明确 phase：
  - pause；
  - propagate；
  - atomic；
  - sweep；
  - finalize。
- `step` 推进真实工作量。
- `setpause`/`setstepmul` 保存并返回正确旧值。
- 接入 allocation debt/threshold。

#### M1.11 Write barrier

- 接入至少以下写入路径：
  - Table key/value/metatable；
  - Function proto/upvalue/env；
  - Upvalue close/set；
  - Userdata metatable/env；
  - Thread caller/state references；
  - Proto child/constants/debug references；
  - registry/global assignment。
- 增加黑对象指向白对象、增量阶段修改、多 collector/cross-state 的定向测试。

#### M1.12 Weak table、finalizer 与 resurrection

- 对齐：
  - weak key；
  - weak value；
  - ephemeron/键值关系；
  - string 行为；
  - userdata finalizer；
  - finalizer 顺序；
  - resurrection；
  - finalizer 再次触发规则；
  - 关闭时 finalizer drain。

#### M1.13 内存与耐久测试

- 增加：
  - object count；
  - accounted bytes；
  - allocator live/peak bytes；
  - 1000 轮 state create/close；
  - coroutine create/drop；
  - weak/finalizer 多周期；
  - closure/upvalue 生命周期；
  - dump/load 后生命周期。
- 对 unsafe 热点运行可行的 Miri、ASan 或等价检查。

### 6.5 M1 完成门槛

- 无已知 collector、coroutine state 或 fixed object 泄漏。
- `collectgarbage` 真实回收并降低对象数/内存。
- shutdown 后 live objects 和 allocator live bytes 为 0。
- ByteString、root set、barrier、weak/finalizer/resurrection 测试全部通过。
- 不存在返回 `&'static mut` 的运行时裸指针辅助函数。

### 6.6 M1 执行台账（2026-07-29）

| 任务 | 状态 | 当前证据与未闭合项 |
|---|---|---|
| M1.1 ByteString 设计 | `completed-local` | RFC 已冻结 arbitrary bytes、NUL、pointer+length、显式 UTF-8/lossy 边界与 C++ 前向采样 hash。 |
| M1.2 GcString/StringPool | `completed-local` | 单一 ByteString payload、byte-key interning、0x00–0xff/NUL/invalid UTF-8/identity/hash 测试通过，旧 text API 静态扫描为零。 |
| M1.3 编译器和宿主边界 | `partial` | lexer/parser/codegen、string/io/package、CLI source 与 chunk name 已迁移并有 raw-byte 双 oracle case；真实 dump/load、未来 C API，以及所有生产 GcString 强制 intern 尚未完成。 |
| M1.4 Runtime owner | `partial` | pinned RuntimeHeap、owner-thread/phase/active guard、main roots 与 scoped parts 已实现；P1 `ObjectId + live table + checked borrow` 已拒绝 foreign/stale/address-reuse handle，LightUserdata 也已拆型。LuaState 仍保存 transitional GC/StringPool backpointer，尚无统一 Heap/allocator/services owner。 |
| M1.5 Coroutine state | `partial` | StateArena 独占 coroutine Box；handle identity/generation、retirement 与关闭前 arena 校验已 fail-closed。`PendingState` 以 exact-id `TEMPORARY_STATE_ROOTS` 保护未发布槽，Drop 回滚并推进/退休 generation；create/wrap 在对象临时根内完成 State↔Thread 双向绑定并仅在直接压栈后提交。sealed `RuntimeNativeFunction`、scoped mailbox、独立 VM exit、deferred C frame、Runtime activation/upvalue transfer stack、rooted transfer seed 和 generic-for continuation 已接入生产 resume/wrap。open Upvalue 现为 `StateHandle + stack index`，远端 GET/SET 通过 Runtime 单-state turns 访问，state drain 在 generation advance/retirement 前关闭节点；reachable Upvalue 也会入队 owner state。精确 C++ `A→B→A` `Normal` 祖先 continuation、protected `pcall`、wrap yield/error 和 suspended-coroutine closure 读写均有回归。main state 仍是 external arena slot，debug/protected-helper 跨 state open-Upvalue、深链/完整 fault 矩阵仍开放。 |
| M1.6 悬垂 registry | `completed-local` | pseudo dump/source thread-local registry 已删除，`string.dump` 在 M3 serializer 前稳定返回 unsupported。 |
| M1.7 Root inventory | `partial` | 24/24 inventory schema 校验通过；canonical 双队列、identity-aware collector 队列、managed `ACTIVE_PROTO/DEBUG_PROTO`、checked `OPEN_UPVALUES` owner、临时对象根 registry、`TEMPORARY_STATE_ROOTS/PendingState`、`COROUTINE_ACTIVATION_BUFFER` seed、compiler Proto→Function、library registration 与 package graph 事务发布已实现。当前为 21 partial、2 implemented（`OPEN_UPVALUES`、`TEMPORARY_STATE_ROOTS`）、1 missing（fixed strings），无 unsafe 条目；IO、VM/app/results 仍阻止 destructive sweep。 |
| M1.8 确定性 shutdown | `partial` | Runtime close/Drop 以 state→非 fixed Thread→其余非 fixed→fixed 顺序释放，object/root/string/queue/state/count 均归零；7 layout、fixed/ordinary DropProbe、open-upvalue close 与 1000 轮耐久测试通过。关闭期 Lua `__gc`、显式 IO/module service drain 和 allocator live bytes 仍是公开 debt。 |
| M1.9 真实 full collection | `pending` | `collectgarbage("collect")` 仍只做兼容 mark/weak/finalizer 路径，不 sweep。 |
| M1.10 Incremental GC | `pending` | phase、debt、pause、stepmul 和真实 step 尚未实现。 |
| M1.11 Write barrier | `pending` | helper 存在但 production mutation 尚未统一接线，部分 raw barrier 在 provenance 基座前仍不安全。 |
| M1.12 Weak/finalizer/resurrection | `pending` | 只有组件级近似；pending-finalizer root、reentrancy、exactly-once、resurrection 与 close drain 未验收。 |
| M1.13 内存与耐久 | `partial` | 1000 runtime/coroutine close 与 DropProbe 已通过；allocator、真实多周期 collection、Miri/ASan 和 binary dump 生命周期待补。 |

本台账中的 `completed-local` 只表示对应子任务的本地实现与定向证据完成；
远程 CI 首次通过和合入前不改为最终 `completed`。M1 整体仍为 `active`。

## 7. M2：源码、标准库与工具行为对齐

### 7.1 编译器与 Proto

#### M2.1 错误处理

- 将 codegen 中的 `panic!` 转为结构化错误。
- 增加：
  - 超长控制结构；
  - 寄存器耗尽；
  - 递归/嵌套限制；
  - 非法 lvalue；
  - malformed method call；
  - 极端常量/局部变量数量。
- 错误 source、line、category 与 C++ 对齐。

#### M2.2 Opcode 与 instruction parity

- 逐项验证 38 opcode：
  - enum order；
  - instruction encoding；
  - RK；
  - metadata；
  - jump range；
  - SETLIST；
  - CLOSURE/upvalue binding；
  - CALL/TAILCALL/RETURN；
  - VARARG。
- 每个 opcode 至少有：
  - 单元测试；
  - Lua 行为测试；
  - bytecode differential。

#### M2.3 Proto 完整性

- 对齐：
  - constants；
  - nested protos；
  - num params；
  - is_vararg；
  - max stack；
  - line info；
  - local ranges；
  - upvalue names；
  - source/chunk name。

### 7.2 VM 语义

#### M2.4 调用与返回

- Lua→Lua、Lua→host、host→Lua、递归和重入。
- 多返回的最终/非最终折叠。
- tail call 和 tail return debug event。
- C function error/yield 边界。
- stack overflow 和调用深度限制。

#### M2.5 Closure 与 upvalue

- open upvalue 唯一性。
- sibling closure 共享。
- scope exit、break、return、tailcall 时关闭。
- coroutine yield/resume 穿越 upvalue。

#### M2.6 Metamethod

- `__index`、`__newindex` 链和循环限制。
- 算术、比较、concat、len、call、tostring。
- table/userdata/primitive metatable。
- 错误消息和 source line。

#### M2.7 Coroutine 与 debug

- create/resume/yield/status/running/wrap。
- 首次参数、yield values、resume values、多返回。
- dead/running/error 状态。
- debug hook call/return/line/count。
- getinfo/getlocal/getupvalue/setlocal/setupvalue/traceback。

### 7.3 标准库

#### M2.8 首批可观察修复

- 真实 stdin/stdout/stderr。
- 注册 `_VERSION`。
- 补齐当前与 C++ 注册表相比缺失的函数：
  - `table.pack`；
  - `table.unpack`；
  - `table.move`；
  - `io.popen`；
  - `os.exit`；
  - `os.getenv`；
  - `debug.debug`；
  - `debug.getmetatable`。
- 建立真正的 string metatable，而不是 VM 硬编码 fallback。

#### M2.9 Base 和错误行为

- assert/error/pcall/xpcall。
- load/loadfile/loadstring/dofile。
- getfenv/setfenv。
- collectgarbage/gcinfo。
- tonumber/tostring/select/unpack。
- 错误对象、错误级别、source line 和 traceback。

#### M2.10 String 与 pattern

- 全部按 bytes 工作。
- pattern class、capture、frontier、balanced、gmatch、gsub replacement。
- format 与 Lua 5.1 边界。
- string.dump 在 M3 前显式标记 unsupported，不保留伪兼容。

#### M2.11 Table

- insert/remove/concat/sort。
- comparator 错误和非破坏性失败。
- foreach/foreachi/getn/maxn。
- C++ 项目扩展函数。

#### M2.12 IO

- 默认流和重定向。
- open/read/write/seek/flush/close/lines/type/tmpfile/popen。
- text/binary mode。
- EOF、错误 tuple 和 closed-file 行为。
- file userdata 类型身份和 `__gc`。

#### M2.13 OS

- clock 使用正确的进程 CPU 时间语义。
- date/time 的 local/UTC、`!`、isdst、格式符。
- difftime、execute、exit、getenv、remove、rename、tmpname。
- setlocale 的真实支持或明确、已批准的平台差异。

#### M2.14 Math

- RNG 状态由 Runtime/GlobalState 持有，禁止进程全局共享。
- random/randomseed 与 state isolation。
- 边界参数、NaN、Inf、范围错误和格式。

#### M2.15 Package

- package.config/path/cpath/loaders/preload/loaded。
- require loader 顺序、循环依赖和错误聚合。
- Lua module、module/seeall。
- native loader 在 M3 接入。

### 7.4 CLI、REPL 与 bytecode 工具

#### M2.16 CLI

- 参数、stdin、`-e`、`-l`、`-i`、`--`、script args。
- 与 C++ 对齐 `--trace`、`--trace-diff` 等项目选项。
- stdout/stderr/exit code。
- hash-bang、二进制输入、文件错误和 source name。

#### M2.17 REPL

- 错误后继续会话。
- 多行和不完整输入。
- 自动表达式和前导 `=`。
- history、meta command、completion、prompt、Ctrl-C。
- State/stack 在每次命令后恢复一致。

#### M2.18 Bytecode 工具

- 错误参数返回非零。
- 严格校验 `--format`。
- text/JSON 正确转义。
- 显示真实字符串常量和 nested Proto。
- full/cfg/diff 模式。
- JSON 使用稳定 schema，禁止手写无效转义。

### 7.5 M2 完成门槛

- 125 个项目 Lua fixture 全部按 manifest 得到预期结果。
- 4-case differential 4/4 通过，或仅剩已批准 deviation。
- Lua 5.1 official strict `all.lua` 通过。
- slow suite 通过。
- 38 opcode bytecode/trace 无未解释差异。
- CLI/REPL/bytecode 进程级测试全绿。

## 8. M3：Binary Chunk、C API 与动态模块

### 8.1 Binary Chunk

#### M3.1 格式合同

- 明确目标：
  - Lua 5.1 官方 chunk 兼容；
  - `lua_cpp` 项目扩展；
  - 端序、数字格式和平台字段。
- 建立官方 `luac`、`lua_cpp`、`lua_rust` 三方 golden。

#### M3.2 Serializer/Deserializer

- header；
- function Proto；
- code；
- constants；
- nested protos；
- debug info；
- source；
- upvalue metadata。
- 不允许进程内 ID 或指针进入持久化格式。

#### M3.3 Bytecode verifier

- 长度和整数溢出。
- recursion/depth。
- instruction/opcode/operand 合法性。
- constant/proto/upvalue 索引。
- stack/register 范围。
- allocation/size limit。
- truncated、random、malformed corpus。

验收：

- `string.dump` 结果可跨进程加载。
- 官方/Cpp/Rust 的允许格式按合同互通。
- malformed chunk 不 panic、不越界、不产生未受控分配。

### 8.2 C API

#### M3.4 Crate 与制品

- 新建 `lua_capi` crate。
- 输出：
  - staticlib；
  - cdylib；
  - `lua.h`；
  - `lauxlib.h`；
  - `lualib.h`；
  - 后续 `lua_runtime.h`。
- 所有公开类型使用明确的 `#[repr(C)]`/opaque handle。
- 禁止 Rust panic 穿越 FFI。

#### M3.5 State、stack 与类型 API

- newstate/close。
- allocator callback。
- stack top/index/checkstack/xmove。
- type/query/conversion。
- push/copy/replace/remove/insert。
- registry、globals、pseudo-index。

#### M3.6 Table、userdata、function 与调用

- get/set/raw/table iteration。
- light/full userdata 和 metatable。
- C closure/upvalues。
- call/pcall/cpcall/error。
- reader/writer/load/dump。
- coroutine resume/yield/status。
- GC/debug/hook。

#### M3.7 lauxlib 与 lualib

- buffer API。
- argument checking。
- reference system。
- loadfile/loadbuffer。
- library registration。
- openlibs。

#### M3.8 C API 验证

- 迁移 `lua_cpp` 的官方 123 项机器合同。
- 纯 C 编译测试。
- 独立 C/C++ consumer。
- static/shared link。
- allocator failure、state isolation、protected error。
- API 与官方 Lua 5.1 差分 probe。

### 8.3 Native module

#### M3.9 package.loadlib

- Windows LoadLibrary/GetProcAddress。
- POSIX dlopen/dlsym。
- module registry 和生命周期。
- symbol/error 文本。
- sandbox capability gate。
- unload/State close 顺序。

验收：

- 官方 C API 123/123 通过。
- public export contract 无缺失/多余符号。
- 独立 native module 可构建、加载、调用和卸载。
- 1000 次 module lifecycle 不泄漏。

## 9. M4：生产运行时、安全与观测

### 9.1 Runtime configuration

- 迁移版本化 `RuntimeConfig`。
- unrestricted 默认和 game-server preset。
- 标准库 capability bitmap。
- 文件、进程、动态模块和运行时编译 capability。
- 创建期限制和每次 execution window 限制分离。

### 9.2 执行治理

- instruction budget。
- native work budget。
- monotonic deadline。
- cooperative cancellation。
- finalizer budget。
- compiler/parser/resource limits。
- coroutine 和重入共享同一执行窗口。

### 9.3 Owner-thread 与状态机

- State 固定 owner thread。
- foreign-thread 只能使用生命周期安全 cancellation handle。
- busy/idle/closed 状态明确。
- close、metrics、配置 API 的调用时机有机器合同。

### 9.4 Allocator 与内存

- 支持 Lua allocator callback。
- 明确已路由和未路由的内存范围。
- 失败必须 transactional：
  - 旧块仍有效；
  - 不部分提交 stack/table/string；
  - 可解除限制后继续使用。
- 进程级边界作为最终兜底，不夸大 callback allocator 能力。

### 9.5 Metrics

- 初始/剩余/消耗 instruction budget。
- native work。
- deadline 和 cancellation。
- stop reason。
- allocator live/peak。
- GC phase/objects/bytes。
- 只在 owner-thread 且 State idle 时提供一致快照。

### 9.6 Production worker

- 只使用公开 C API。
- 组合 allocator cap、进程 CPU/内存边界、timeout、输出限制。
- 输出稳定 JSON 结果分类。
- 边界安装失败时 fail closed。
- 覆盖 success、instruction、timeout、cancel、allocator、process resource。

### 9.7 安全测试

- parser fuzz。
- bytecode verifier fuzz。
- stdlib numeric/path fuzz。
- ASan/UBSan/TSan 或 Rust 对应检查。
- unsafe hotspot Miri。
- cancellation/finalizer/state-close races。
- sandbox denied paths。

### 9.8 M4 完成门槛

- 与 C++ runtime public API 的结构和行为合同对齐。
- 所有预算、deadline、取消和 sandbox 拒绝路径有进程级测试。
- 1000 轮 runtime soak 无泄漏、无挂起、无重复 finalizer。
- worker 的成功和各类故障结果稳定。

## 10. M5：跨平台 SDK、性能与发布

### 10.1 跨平台

- 删除 workspace 默认强制 `x86_64-pc-windows-msvc`。
- stack/linker 参数放入 target-specific 配置。
- CI 至少覆盖：
  - Windows MSVC Debug/Release；
  - Linux stable/MSRV；
  - macOS；
  - 必要的 ARM64 lane。
- 规范化平台允许差异。

### 10.2 覆盖率

- 为以下组件建立硬阈值：
  - bytecode verifier；
  - C API；
  - GC phases；
  - opcode handlers；
  - parser/codegen；
  - sandbox denied paths。
- 阈值下降需要显式批准。

### 10.3 性能

- 建立与 C++ 同机配对 benchmark：
  - VM instruction throughput；
  - host→Lua；
  - Lua→host；
  - coroutine；
  - closure/upvalue；
  - GC pause/p99/max；
  - parser/codegen；
  - stdlib 热路径。
- 正确性完成前只记录，不用性能差异阻塞语义修复。
- 稳定后设置绝对 SLO 和相对回归预算。

### 10.4 SDK 与发布制品

- 安装头、静态库、共享库、Cargo artifacts。
- C/C++ 外部 consumer。
- 版本和 ABI metadata。
- SPDX SBOM。
- SHA-256 manifest。
- archive traversal/篡改检查。
- 解压后重新构建并执行 consumer。

### 10.5 发布治理

- release checklist。
- immutable RC tag。
- 同 SHA 全平台证据。
- nightly endurance。
- benchmark、coverage、fuzz、sanitizer。
- 目标环境 shadow/canary 和回滚演练。

### 10.6 M5 完成门槛

- 所有目标平台使用同一提交构建和验证。
- SDK consumer、SBOM、checksum 和归档验证通过。
- 性能无未批准的重大回归。
- release checklist 全部满足后才创建 RC。

## 11. CI 分层

### 11.1 PR 快速门

- fmt；
- clippy `--all-targets -D warnings`；
- Debug workspace tests；
- Release 核心测试；
- rustdoc `-D warnings`；
- basic/control-flow/functions/regressions；
- 4-case differential；
- 代表性 bytecode/VM trace；
- deviation/doc drift。

目标：在合理时间内阻止绝大多数语义和工程回归。

### 11.2 Nightly

- 全量 Lua fixture；
- official strict/slow；
- 全量 bytecode/VM trace；
- C API differential；
- sanitizer/Miri；
- fuzz 定时运行；
- coverage；
- runtime/module soak；
- benchmark 采样。

### 11.3 Release

- 全平台 Release。
- 123/123 C API。
- official suite。
- production worker 故障注入。
- 1000+ 轮 soak。
- SDK consumer。
- SBOM/checksum/archive。
- 绝对 SLO 和相对性能门。

## 12. 每个任务的完成定义

每个迁移任务必须包含：

```yaml
id: Mx.y-short-name
cpp_oracle:
  commit: 87c15e6
  files: []
rust_targets: []
lua_cases: []
unit_tests: []
differential:
  official_lua: required | not_applicable
  lua_cpp: required | not_applicable
unsafe_changes:
  count: 0
  proofs: []
acceptance: []
docs: []
known_deviations: []
```

PR 完成条件：

1. 有 C++ 参考位置和可观察行为说明。
2. 新增或更新失败测试。
3. Debug/Release 和适用的差分全部通过。
4. unsafe 块有完整 SAFETY 证明。
5. 没有新增未登记模拟或 unsupported 行为。
6. phase report、type mapping、glossary、deviation log 按需更新。
7. 工作树和生成 artifact 清理策略明确。

## 13. 推荐的首批 PR

PR-1 至 PR-3 的 M0 工作范围已在当前工作树完成；是否按三个独立提交/PR
拆分由合入策略决定。PR-4 和 PR-5 是当前 M1 主线，PR-6 仍严格依赖 PR-5。

### PR-1：质量门修复

范围：

- 修复 nextest 递归 alias。
- all-targets clippy。
- rustdoc `-D warnings`。
- Debug/Release test。
- 清理 104 条 lint 和 19 条文档 warning。

不得混入解释器语义修改。

### PR-2：进程 runner 与可观察 IO

范围：

- Lua fixture manifest。
- CLI 子进程 runner。
- 真实 stdin/stdout/stderr。
- `_VERSION`。
- 4-case differential。

目标是先获得可靠失败报告，不要求一次修完全部差异。

### PR-3：恢复 bytecode/VM trace parity

范围：

- 恢复比较脚本。
- 固定 C++ oracle。
- 建立结构化 artifact。
- 选择最小代表用例。

### PR-4：ByteString

范围：

- RFC。
- GcString/StringPool。
- loader/lexer/parser。
- string/io 边界。
- 任意字节和 UTF-8 回归测试。

这是大范围架构 PR，不与 GC 或 C API 混合。

### PR-5：Runtime 所有权和 deterministic shutdown

范围：

- Runtime owner。
- coroutine arena/handle。
- root inventory。
- 删除 `&'static mut` 辅助。
- collector/fixed object/State 关闭。
- 暂不启用自动 sweep。

### PR-6：真实 full GC

范围：

- full collect/sweep。
- 真实内存计数。
- collectgarbage/gcinfo。
- weak/finalizer/resurrection。
- write barrier。
- soak。

PR-5 完成前不得合并 PR-6。

### PR-7：源码兼容收敛

范围：

- 以差分报告为队列逐项修复。
- 优先处理调用、upvalue、coroutine、metamethod、错误。
- 每个问题保留最小回归 case。

## 14. 风险清单

| 风险 | 影响 | 缓解措施 |
|---|---|---|
| 在 root/所有权修复前启用 sweep | UAF、随机崩溃 | 强制 PR-5 前置，root inventory + soak |
| ByteString 改动面过大 | 编译器、stdlib、CLI 同时回归 | 独立 RFC/PR，双表示过渡必须有截止任务 |
| C++ `main` 持续变化 | 永远追不上目标 | 固定 SHA，只允许 baseline PR 升级 |
| 596 个内部测试造成错误安全感 | 近似实现被误判完成 | 以进程差分和 official suite 为完成门 |
| 模拟 GC 测试自证 | 测试绿但不回收 | 使用对象数、真实字节和析构 side effect |
| FFI panic/alias | 进程 abort 或 UB | catch boundary、opaque handle、Miri/sanitizer |
| 强制 Windows target | Linux/mac 无法验证 | 移除默认 target，建立平台矩阵 |
| 一次 PR 跨多个基础层 | 难审查、难定位回归 | 按首批 PR 边界拆分 |
| 性能优化提前 | 行为偏移、复杂度上升 | parity 完成前性能只记录不主导设计 |

## 15. 进度报告方式

不再使用“文件已存在”或“函数已注册”作为完成指标。每周状态应按以下格式报告：

| 领域 | 通过合同 | 总合同 | 未解释差异 | P0 blocker | 趋势 |
|---|---:|---:|---:|---:|---|
| CLI behavior |  |  |  |  |  |
| Bytecode |  |  |  |  |  |
| VM trace |  |  |  |  |  |
| Official Lua |  |  |  |  |  |
| GC/lifecycle |  |  |  |  |  |
| Standard library |  |  |  |  |  |
| C API |  |  |  |  |  |
| Production runtime |  |  |  |  |  |

每个阶段只在对应完成门槛全部满足后标记完成。若仍有 expected failure，必须列出 deviation ID、
维护者、原因和计划移除版本。

## 16. 下一步

M0 已在本地完成，M1 基础层已推进到 coroutine activation trampoline 收口。下一步按依赖
顺序执行：

1. 已完成 LightUserdata 拆型与 `ObjectId + collector live table`；继续保持
   live sweep 禁用，直到 `Value::String` 内容 Eq/Hash、执行层 scoped borrow
   和唯一 Heap owner 都闭环。
2. top-level/active/debug Proto 已全部改为受管 `GcRef<Proto>`；
   RuntimeId/StateHandle issuance 与 slot generation exhaustion 也已
   fail-closed；scoped mailbox、`VmExit::NativeRequest`、deferred C frame
   与 activation stack 已把生产 resume/wrap 接入 Runtime 调度，并保持每
   turn 单 state 借用和 release-before-switch。open Upvalue 已改为
   `StateHandle + stack index`，远端读写、owner-state root 入队和
   close-before-generation-advance 均已接入；debug/protected-helper 跨
   state 访问仍作为后续矩阵。
3. lexical temporary-object registry、HRTB `PublicationTxn/Rooted`、panic/nested
   cleanup、mark seed 与 `TEMPORARY_STATE_ROOTS/PendingState` 已完成；
   coroutine create/wrap 的 State→Thread→stack、compiler 内部 string/child
   Proto builder、顶层 Proto→Function 以及 CLI/bytecode/base/package loader
   handoff、全部标准库注册和 package/loaded/preload/nested-module graph 均已
   事务化。下一步按 IO → VM/app/results 顺序迁移；pending finalizers 与
   fixed strings 仍须进入 canonical roots。
4. 建立唯一 `Heap` owner，移除 LuaState GC/StringPool backpointer 与
   standalone collector/pool 错配，然后实现 Runtime-only、Thread/state
   prepass 的 stop-the-world full collection。
5. full collection、weak/finalizer/resurrection 全部通过后才替换
   `collectgarbage` 模拟计数；production mutation 全部接线后再做
   incremental phase/debt/barrier。
6. managed owner/publication-root 树稳定后重跑完整 M0/M1 gate、raw-byte 双 oracle、fresh
   101-case non-official 与代表性 bytecode parity，并更新精确 artifact/
   SHA；远程 CI 首次运行必须实际执行 cargo audit。
7. M2 backlog 中的 88 个进程差异与 VM trace unsupported 继续作为显式输入。
   原两个 bytecode case 的 opcode/constant/metadata 已与 C++ 一致，各只剩
   `localNames` missing-evidence + value 两项（C++ printer 不输出 locals）；
   `test_closure_pipeline.lua` 仍达到 500 条真实差异上限。不得用
   normalization 静默消除任一缺口。

在上述 M1 硬门全部关闭前，不开始正式 C API、binary chunk、动态模块，也不
把 core 单元测试中的 `sweep` 接到 live VM。

### 16.1 当前续接检查点（2026-07-29）

本轮收口时已经完成并验证：

- P1 `ObjectId + collector live table` provenance、LightUserdata 拆型、持久
  root/weak/finalizer/external 队列身份校验；
- managed active/debug Proto、frame return/error/unwind 清理与 canonical
  roots；运行时 `*const/*mut Proto` 静态命中为 0，collector tag dispatch
  两处强转除外；
- deterministic Runtime shutdown 基础和 1,000 轮归零；
- 临时对象/状态根：`PublicationTxn/Rooted`、compile-fail 防逃逸、nested/panic
  cleanup、mark seed、1,000 scope zero，以及 exact-id `PendingState` 的
  rollback/commit、独立 state root seed 和 coroutine create/wrap 直接压栈发布；
  compiler 的 interned string/child Proto/top Proto/Function 也已在同一事务
  中构造，并由 explicit root 或活动 Lua stack 接管；标准库 Table/key/
  C-or-runtime-native Function 注册与 package 对象图也已由 traced Table/stack
  接管；IO、VM/app/results 尚未迁移；
- bytecode schema v2 和首个指令形状切片。证据位于
  `target/compatibility/bytecode-schema-v2-original-two/report.json` 与
  `target/compatibility/bytecode-closure-schema-v2/report.json`（target
  artifact 不提交）。
- StateHandle identity/generation 前置切片：并发 checked RuntimeId、
  non-Clone issuer、raw 构造 compile-fail、MAX-generation retirement、
  free-list/count preflight 与真实 stale/foreign trace 均有回归。
- Runtime coroutine activation trampoline：sealed runtime-native
  resume/wrap、scoped mailbox、独立 VM exit、deferred C frame、activation
  stack、generic-for continuation 与 canonical activation-buffer root seed
  已接入生产 app/stdlib 入口；caller borrow 在 target resolve 前释放。
- open-Upvalue owner：`LuaState` 维护按栈索引降序且去重的 checked
  `GcRef<Upvalue>` 集合，Upvalue 仅保存 `StateHandle + stack index`；
  suspended coroutine closure 的远端 GET/SET 经 Runtime owner turn
  完成，root tracer 可从 reachable Upvalue 入队 owner state，arena drain
  会在 handle generation advance/retirement 前关闭节点。Normal 祖先重放
  上下文也能跨 Upvalue turn 保持。
- 固定 C++ `Normal` 祖先重入 characterization 已独立锁定：
  C++ stdout SHA-256 为
  `bad37c42fcfd369f22fdc9d9ec8d1ce46caaa2e8fa755fe03a41a6e91b2591d2`，
  stock 为
  `0488432cb01117da75f229ab0f43bd1c1ea174853ebde5f1ab62853265f805f6`；
  Rust process regression 已逐字节匹配固定 C++ 输出；该 manifest 仍是
  non-gating characterization、无 approved deviation，因为 stock Lua 行为
  仍与项目目标不同。
- 最新验证：fmt/check、758 个 workspace tests、all-targets Clippy、
  warning-free rustdoc、24/24 root inventory、差分比较器与 parity runner
  自测全部通过；fixture manifest 校验为 131 项；两个 coroutine oracle
  各重复 3 次通过精确字节校验。characterization checker 已同时通过
  Windows PowerShell 5.1 与 PowerShell 7，并将实际执行文件的 SHA-256
  绑定到现有 C++/Lua 5.1.5 provenance build reports。M1 smoke 汇总为
  `checksPassed=true`、`hardFailures=[]`；因显式 smoke/audit skip，按策略
  保持 `foundationPassed=false`，不代表 M1 完成。

下次不要从头审计，按以下顺序继续：

1. 已完成 StateHandle 前置切片：checked/concurrent RuntimeId allocator、
   non-Clone issuer namespace、raw 安全构造 compile-fail、generation
   `u64::MAX` 最后一代与永久 slot retirement、free-list/count shutdown
   preflight；不存在 `fetch_add`/`wrapping_add` 回绕路径。
2. 已完成 coroutine trampoline 生产切片：Runtime 每 turn 单借用、
   release-before-switch、scoped mailbox、`VmExit::NativeRequest`、
   deferred C frame 与 activation stack 已接入 app/stdlib；固定 C++ 允许
   `Normal` 祖先重入及 continuation 二次执行的精确输出已锁定。
3. 已完成 open Upvalue 的
   `Open { owner: StateHandle, index }`、非 intrusive state-owned 集合、
   reachable owner-state 入队、远端 Runtime turn 和
   close-before-generation-advance；debug/protected-helper 跨 state
   行为保留为显式后续矩阵。
4. 已完成 `TEMPORARY_STATE_ROOTS/PendingState`：未发布状态可独立追踪，
   插入/绑定/压栈故障会 exact-id 回滚，`u64::MAX` 仅退休一次；create/wrap
   已用 typed Thread/Upvalue/Function publication 接入生产路径。
5. 已完成当前第一开发项：`CodeGenerator` 的 sealed publication allocator
   在 builder 全程保护 interned strings 与 child Proto，顶层 Proto→Function/
   environment 通过 typed API 构造；CLI、bytecode tool、base load/dofile 与
   package file loader 已在 explicit root 或活动 Lua stack 接管后才释放临时根。
   第二开发项 library/package 也已完成：共享注册层保护 destination Table、
   canonical key、C/Runtime-native Function 与 environment；catalog library
   Table、package/loaded/preload、嵌套 module path、metadata/metatable 与错误
   string 仅在 traced Table/stack 接管后释放。下一开发项是 IO construction
   graph，之后推进 VM/app/results；继续维持 allocation-triggered collection
   禁用。
6. 完成所有生产字符串 canonical interning 或 scoped Eq/Hash，建立唯一 Heap
   owner后，才接 Runtime-only full collection；incremental/barrier/weak/
   finalizer 最后推进。
7. M2 并行下一步是为固定 C++ bytecode printer 的 local-name 证据建立受审计
   方案，并从 closure artifact 的第一个 Proto/PC 差异开始修，不能直接处理
   截断后的 500 条列表。

推荐恢复后先运行：

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets --quiet
cargo clippy --workspace --all-targets -- -D warnings
pwsh -NoProfile -File tools/check_gc_root_inventory.ps1
pwsh -NoProfile -File tools/check_coroutine_normal_ancestor_characterization.ps1
pwsh -NoProfile -File tools/run_lua51_differential.ps1 -ComparatorSelfTestOnly
```
