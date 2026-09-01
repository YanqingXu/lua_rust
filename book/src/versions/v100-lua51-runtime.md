# v1.0 · Lua 5.1 语言运行时兼容收口

<div class="lesson-meta"><span>核心增量：从“有实现”变为“有证据的发布合同”</span><span>6-crate workspace</span><span>checkpoint v1.0</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>让每个兼容结论都有可追溯证据</strong>
  <p>前置：v0.18 的语言、CLI 和观察协议。新知识：闭世界矩阵、双向引用校验、schema
  version、oracle provenance、差分子进程与 release gate。完成后你应能区分代表性通过、
  partial deviation 和被明确排除的版本边界。</p>
</div>

v0.18 已经能运行脚本、模块和协程，也能观察 bytecode、CFG 与 trace。但功能数量不是兼容性证据：如果测试没有说明参照物、平台归一化规则和允许差异，“测试通过”仍可能只是与自己一致。本版完成三件事：**补齐核心源码语法缺口、冻结可观察协议、让每项兼容结论都能追溯到测试或偏差 ID**。

<a id="learning-predict"></a>
## 先预测再运行

判断以下证据各能证明什么：一个 golden output、五个 official-oracle fixtures、38 项语义
映射、完整官方测试套件。它们强度不同，不能互相冒充。再为 complete/partial/excluded
各写一行能力矩阵，检查是否需要 test evidence、deviation ID 或 future-version boundary。

<a id="learning-code-map"></a>
## 代码阅读地图

1. capabilities.tsv：闭世界列出功能、状态、证据和 deviation；
2. deviations.tsv：为 partial 项登记范围、处置和目标版本；
3. release tests：双向检查 capability↔deviation，不允许孤儿或假 complete；
4. versioned JSON/trace envelope：冻结 consumer 可观察协议；
5. Lua51Opcode audit：覆盖官方 ordinal，但明确不是 binary compatibility；
6. oracles.toml：固定版本、commit、URL 与 hash；
7. differential runner：三个真实进程、唯一归一化、timeout 与 lane 结果；
8. release gate：本地质量门与可选外部 oracle 证据分层报告。

本版阅读顺序从证据到实现：先问“这个结论由什么证明”，再进入 parser/opcode/tool 代码。
测试存在并不自动成为证据；只有覆盖对象、参照物和允许归一化都明确时，才能支撑对应行。

## 先定义“完成”

v1.0 使用闭世界模型管理发布范围：

```text
能力矩阵中的每一行
├── complete ──> 必须指向自动测试，不得绑定偏差
├── partial  ──> 必须绑定已登记偏差
└── excluded ──> 必须绑定 v1.1+ 的明确边界

偏差注册表中的每个 ID ──> 必须反向出现在能力矩阵
```

这两个方向都由 `tests/lua51_release.rs` 检查。删除能力引用、拼错 ID、给 `complete` 项偷偷绑定例外，都会让普通 `cargo test` 失败。能力真相源位于：

- `compatibility/capabilities.tsv`：功能、状态、测试证据与偏差 ID；
- `compatibility/deviations.tsv`：范围、处置状态、目标版本和精确描述；
- `compatibility/oracles.toml`：外部参照物的版本、URL、commit 与 SHA-256；
- `compatibility/differential-baseline.json`：已经实际得到的双差分结果。

选择 TSV 是为了让 Rust 测试无需第三方解析库也能执行 closed-world 校验；选择 TOML/JSON 保存 provenance 与结果，是因为字段层级更自然，也方便外部工具读取。

## 补齐 Lua 的三种调用形式

Lua 5.1 的函数参数不只有括号形式：

```lua
consume("lua")
consume "lua"
consume { answer = 42 }
```

v0.18 的 `postfix` parser 只在遇到 `(` 时生成 `Call`。v1.0 把 `(`、字符串 token 和 `{` 都视为参数起点；table constructor 和 string 会包装成单一实参。语句入口也接受括号 prefix expression，因此 `(function() ... end)()` 可以作为独立调用语句。

这处改动揭示一个 Lua 边界：换行不是 statement terminator。下面的 `(` 可能成为上一行表达式的调用后缀，因此需要分号消除歧义：

```lua
local result = 0;
(function() result = 42 end)()
```

端到端测试同时执行三种参数形式、括号调用语句和多返回，证明改动最终穿过 Codegen 与 VM，而不只是 AST snapshot 正确。

## 38 Opcode：语义覆盖不等于二进制兼容

Lua 5.1 `lopcodes.h` 定义 38 个 Opcode。教学 VM 从 v0.5 起使用自己的 32 位寄存器格式，v0.18 已演进为 39 个更显式的 Opcode：

| Lua 5.1 Opcode | 教学 ISA | 原因 |
|---|---|---|
| `LOADBOOL` / `LOADNIL` | `LoadK` | boolean 与 nil 都进入普通常量表 |
| `SELF` | `GetTable + Move` | 方法查找和 self 布置显式展开 |
| `TESTSET` | 条件跳转 + `Move` | 短路复制由 codegen 明确表达 |
| `TFORLOOP` | `Call + JumpIfNil + Move` | iterator 协议拆成可观察步骤 |
| `SETLIST` | `SetList` / `SetListOpen` | 固定宽度与 MULTRET 分开 |

`lua_bytecode::compatibility` 按官方顺序定义 `Lua51Opcode`，并为全部 38 项给出非空映射、direct/composite 标记与说明。测试验证 ordinal 0–37 没有重复或缺项。运行：

```powershell
cargo run --manifest-path `
  book/code/v100-lua51-runtime/crates/lua_bytecode/Cargo.toml -- `
  --format opcode-audit
```

输出 schema 为 `lua-rust-lua51-opcode-audit/v1`，同时固定 `binary_chunk_compatible=false`。源码语义覆盖属于 v1.0，Lua 5.1 binary chunk 编解码与 verifier 属于 v1.1。

## 冻结 bytecode 与 trace 工具协议

v0.18 的 JSON 有稳定形状但没有版本号。本版给 Proto JSON 增加 `lua-rust-bytecode/v1`，给 VM trace 增加 `lua-rust-vm-trace/v1` 顶层 envelope：

```json
{"schema":"lua-rust-vm-trace/v1","events":[...]}
```

byte string 仍以 hex 输出，NaN/Infinity 仍不能冒充 JSON number。进程测试锁定 schema、错误退出码和 38-op audit 的行数。添加兼容字段可保持 v1；改变既有字段语义则必须升级 schema。

## 双 oracle 不是“双份 expected string”

官方 Lua 5.1.5 是语言参照，固定 C++ commit 是工程迁移参照。身份固定为：

- Lua 5.1.5 source SHA-256：`2640fc56...b0738695333`；
- official tests SHA-256：`49e4ca65...67f18ad`；
- C++ oracle commit：`87c15e69...635711`；
- exact-commit codeload archive SHA-256：`66c9506a...1bbfb8`。

`tools/run_differential.ps1` 对每个 fixture 启动三个真实进程，只做已声明的 CRLF→LF 归一化，然后比较 stdout、stderr 与 exit code。每个进程有十秒 timeout，超时只终止该 child process tree。

本次实际基线为 5/5 fixture 同时匹配两个 oracle：

| Fixture | 覆盖 | 结果 |
|---|---|---|
| `language-core` | 循环、多返回、短路、拼接 | 双 lane 通过 |
| `byte-strings` | NUL、0xff、byte/sub/length | 双 lane 通过 |
| `metatables` | `__index` 与 `__call` | 双 lane 通过 |
| `coroutines` | yield/resume 多值与状态 | 双 lane 通过 |
| `core-stdlib` | math/string/table 代表接口 | 双 lane 通过 |

baseline 只证明这五个可观察合同，不等于官方全量测试套件通过。依赖 C API、binary chunk、native module 或尚登记标准库差异的部分不能被静默过滤；能力矩阵必须保持 `partial`/`excluded`，直到对应差异真正关闭。

## Release gate

普通快照验收不依赖外部下载：

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --workspace
```

完整本地发布门执行 Debug/Release 测试、严格 Clippy、示例 stdout、38-op audit；提供 oracle 路径后再执行双差分：

```powershell
book/code/v100-lua51-runtime/tools/release_gate.ps1 -RunOracles `
  -OfficialLua path/to/lua51.exe `
  -CppLua path/to/lua_app.exe `
  -CppRoot path/to/pinned-source
```

不传 `-RunOracles` 时脚本明确打印 skipped，而不是把缺少 oracle 当作通过证据。

## Rust 知识映射与设计选择

- `#[repr(u8)]` 和固定数组把官方 opcode ordinal 变成可编译的不变量；
- `&'static [Opcode]` 表达一个官方语义由多个教学指令组合；
- `BTreeMap/BTreeSet` 让 registry 的重复、引用和顺序检查确定；
- versioned envelope 把 Rust struct 演进与 JSON consumer 解耦；
- 子进程异步读取 stdout/stderr，避免管道缓冲导致 differential deadlock；
- source/archive hash 与 commit 一起固定，避免 oracle 随网络分支漂移。

替代方案是把教学 ISA 强行改成官方 38 条并立即读取官方 chunk。这会把语言收口与二进制格式、端序、数字宽度、安全 verifier 混成一次破坏性迁移，因此留给 v1.1。另一个方案是只保存 golden output；真实双进程差分能在平台和错误合同改变时暴露差异，证据更强。

## 从 v0.18 迁移了什么

- 新增：Lua string/table call sugar 与 parenthesized prefix statement；
- 修复：help 不再在 option parser 内调用 `process::exit`，而是正常 action；
- 冻结：bytecode JSON 与 VM trace v1 schema；
- 新增：官方 38 Opcode enum、完整语义映射和机器可读 audit；
- 新增：能力矩阵、偏差注册表、oracle manifest 与 verified baseline；
- 新增：5 个双 oracle fixture、provenance-aware runner 和 release gate；
- 保留：v0.18 的语言、VM、GC、Coroutine、标准库、模块、CLI 与工具测试；
- 未伪装：标准库剩余差异和 v1.1+ 能力继续显式登记。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v100-lua51-runtime/Cargo.toml -- `
  book/code/v100-lua51-runtime/examples/demo.lua
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --workspace
```

示例输出：

```text
100
```

Release gate 还要求相同测试在 `--release` 下通过、Clippy 零 warning、示例 stdout 精确匹配，并检查 38-op audit 没有宣称 binary compatibility。

## 本版本暂不支持

- Lua 5.1 binary chunk、`string.dump` 与官方 opcode 二进制编码（v1.1）；
- Lua C API/ABI、headers、lauxlib 和外部 C consumer（v1.2）；
- `package.loadlib` 与动态 native module（v1.3）；
- string pattern 全集、完整 `string.format`、函数 environment API；
- io/os/package/debug 的全部平台与边缘 API；
- C runtime `rand` 序列、完整 locale/timezone/DST 合同；
- Windows argv/path 的无损宽字符 adapter；
- readline、signal-aware REPL、沙箱、配额和跨平台 SDK。

后三组 v1.0 范围内差异分别由 `LUA51-D004`–`LUA51-D011` 登记；它们不是未声明通过项。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>选择一个 composite opcode，从源码行为、教学 bytecode 与 VM trace 三层描述证据，并
  解释为什么仍不能读取官方 binary chunk。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>给 capability evidence 增加内容 hash，检测文件存在但证据漂移；同时为错误 fixture
  设计不依赖绝对路径的分类 schema。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>故意删除一条 deviation 反向引用或给 complete 项绑定 deviation，运行 release test，
  沿失败信息验证闭世界检查能准确指出非法关系。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

读者应能解释：为什么双 oracle 比单个 expected output 强；为什么“38 项语义映射”不等于“binary chunk 兼容”；为什么偏差注册表必须双向闭合；以及为何外部 oracle 缺失必须显示为 skipped 而不能假绿。

## 与成熟实现和专题章节的对应

- 成熟实现的 differential runner、M0/M1 gate、official source manifest 和 living deviation log 是本版最小发布治理的扩展目标；
- [Bytecode](../ch03-compiler/bytecode.md) 对照教学 ISA 编码；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 对照 composite opcode；
- [用测试守住兼容性](../ch05-engineering/testing.md) 深入 fixture、oracle 与进程级证据；
- 根目录 `book_plan.md` 继续定义 v1.1–v1.4 路线。
