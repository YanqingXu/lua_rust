# v0.18 · CLI、REPL 与字节码工具

<div class="lesson-meta"><span>核心增量：可自动验收的用户界面与观察工具</span><span>6-crate workspace</span><span>checkpoint v0.18</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把终端行为和观察工具变成稳定协议</strong>
  <p>前置：v0.17 的共享 Session、loader 与宿主 I/O。新知识：Action enum、终端检测、
  BufRead 状态、稳定 JSON/CFG、子进程 timeout。完成后你应能从真实进程测试证明
  stdout/stderr/exit-code 和 REPL 状态合同。</p>
</div>

到 v0.17 为止，binary 只读取一个文件并总是打印 Rust 风格的最终 Value。这足以演示 VM，却不是 Lua CLI 合同，也无法稳定观察编译器和执行状态。本版增加一个核心能力：**把脚本、交互和诊断都变成有明确 stdout/stderr/exit-code 合同的工具，并用真实子进程测试守住它们**。

<a id="learning-predict"></a>
## 先预测再运行

为 `lua_app -e "x=1" script.lua arg` 写出 action 顺序和 `_G.arg` 索引。再分别预测未知 option、
脚本运行时错误、help 的 stdout/stderr/exit code。最后判断 REPL 每行新建 VM 会破坏哪些状态：
global、package cache、registry、coroutine 和 GC 都会丢失。

<a id="learning-code-map"></a>
## 代码阅读地图

1. CLI parser：argv → 有序 `Vec<Action>`；
2. `Session`：唯一持有跨 action/script/REPL 的 VM；
3. arg-table builder：以 script position 为零点计算负/正索引；
4. result boundary：Lua output、diagnostic 和 usage 映射到不同 stream/code；
5. REPL buffer：complete/incomplete/error 三类编译结果；
6. bytecode JSON：稳定表示 bytes、非有限 number 和 nested Proto；
7. CFG builder：leaders → basic blocks → edges；
8. process runner：spawn、关闭 stdin、并行 drain、deadline、kill exact child。

本章的 Rust 学习点不是“会用 Command”，而是把一次进程执行当成资源协议：stdin 必须及时
关闭，stdout/stderr 必须都被 drain，timeout 后必须 kill 并 wait，才能避免测试自身死锁或
遗留 child。

## CLI 是动作序列，不是一个脚本参数

命令行 parser 先把 options 转成按原顺序执行的 actions：

```text
-e chunk ─┐
-l module ├──> shared Session/VM ──> script or stdin ──> optional REPL
-v        ┘
```

支持的入口包括：

- `lua_app script.lua args...`：运行文件；
- `lua_app -` 或无参数且 stdin 非终端：从 stdin 读取 bytes；
- `-e stat` / `-estat`：可重复执行 chunk；
- `-l name` / `-lname`：按出现顺序调用 require；
- `-i`：actions/script 完成后进入同一个 VM 的 REPL；
- `--`：终止 option parsing，使以 `-` 开头的脚本名合法；
- `-v/--version`、`-h/--help`；
- 保留教学兼容开关 `--dump-bytecode`，正式观察推荐独立工具。

Session 只创建一次 VM，因此多个 `-e`、preload module、脚本和 REPL 能看到彼此的 globals、package cache、registry 和 GC 状态。

## `arg` table 的相对索引

CLI 保存包括 executable 在内的原始 argv，并以 script 位置为原点：

```text
lua_app -e setup script.lua first second
   -3    -2   -1       0        1     2
```

具体 option operand 的位置会随 argv 改变，但规则固定为 `arg[index - script_position]`。脚本路径放在 0，脚本参数从 1 开始，解释器/options 位于负索引。VM 的 `set_arg_table` 创建真正 Heap table，再写入 `_G.arg`；测试同时读取正索引和 `arg[-1]`。

Windows `OsString` 在当前教学 checkpoint 以 lossy UTF-8 注入 Lua，任意宽字符 argv 留给跨平台 SDK 收口；脚本文件本身仍按 bytes 读取。

## stdout、stderr 与退出码

CLI 不再自动打印脚本 return value。Lua `print/io.write` 决定 stdout，REPL 才显示求值结果：

| 情况 | stdout | stderr | exit code |
|---|---|---|---:|
| 正常脚本 | Lua 自己的输出 | 空 | 0 |
| compile/runtime/file error | 已产生的 Lua 输出 | `lua: message` | 1 |
| 未知 option/缺 option operand | 空 | error + usage | 2 |
| help/version | 合同文本 | 空 | 0 |

区分 1 和 2 让 shell/CI 能判断“用户程序失败”还是“调用方式错误”。进程测试还验证脚本创建的文件在进程退出后真实存在且内容正确，不只比较 console 文本。

## 持久 REPL 与续行

REPL 复用 Session，支持：

- `= expression` 自动改写成 `return expression`；
- 非空 return values 以 tab 分隔，byte string 直接写 stdout；
- compile error 若表示缺少 `end`、unexpected end 或未终止字面量，则保留 buffer 并显示 `>> `；
- 普通 compile/runtime error 写 stderr、清空当前 buffer，REPL 继续；
- EOF 干净退出。

进程测试输入 assignment、表达式缩写和三行 if block，验证 global state 没有在每行后重建 VM。

## `lua_bytecode` crate

v0.18 新增第六个 crate。它不修改 Proto，只提供四种观察视图：

```powershell
cargo run --manifest-path book/code/v18-cli-tools/crates/lua_bytecode/Cargo.toml -- `
  --format text script.lua
cargo run --manifest-path book/code/v18-cli-tools/crates/lua_bytecode/Cargo.toml -- `
  --format json script.lua
cargo run --manifest-path book/code/v18-cli-tools/crates/lua_bytecode/Cargo.toml -- `
  --format cfg script.lua
cargo run --manifest-path book/code/v18-cli-tools/crates/lua_bytecode/Cargo.toml -- `
  --format trace script.lua
```

### Text 与稳定 JSON

text 复用 core disassembler。JSON 递归输出 Proto name、stack/params/vararg、constants、每条指令的 raw/pc/line/op/a/b/c/bx/sbx 和 nested protos。

```rust
{{#include ../../code/v18-cli-tools/crates/lua_bytecode/src/lib.rs:bytecode_json}}
```

Lua string constant 可能包含 NUL 或无效 UTF-8，所以 JSON 不把它伪装成 Unicode text，而是输出 `{"type":"string","hex":"..."}`。NaN/Infinity 也编码为 string，避免生成非法 JSON number。

### CFG

CFG builder 先找 basic-block leaders：入口、jump target、jump/return 后的下一条指令；再为每个 block 输出 DOT node 和 jump/next edge。

CFG 构建分两遍：第一遍收集入口、jump target、终止指令后继作为 leaders；第二遍切分 block
并按 opcode 添加 jump/fallthrough edge。
[查看完整 CFG builder](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v18-cli-tools/crates/lua_bytecode/src/lib.rs)

Jump target 必须落在 code 范围并恰好对应 leader，否则返回 bytecode error。ForPrep/ForLoop 和条件跳转分别建模，不根据源码缩进猜控制流。

### Trace

VM 默认不记录 trace。调用 `enable_trace` 后，dispatch 在每条指令执行前保存：step 顺序、Lua call depth、Proto name、pc、line、opcode 和 register snapshot；`take_trace` 关闭记录并转移 events。

trace tool 先编译并执行 Proto，runtime error 仍以 exit 1 报告；成功时输出 JSON array。这样可以区分“字节码是什么”和“这次输入实际走了哪些状态”，为 v1.0 的 oracle VM trace gate 准备稳定数据形状。

## Fixture manifest 与超时 runner

`tests/fixtures/manifest.tsv` 为每个 CLI case 声明 script、args、expected exit、完整 stdout 和 stderr substring。Runner 启动真实 `lua_app` binary，连接三个 stdio stream，并轮询 `try_wait`：

```text
spawn ──> write/close stdin ──> poll until deadline
                              ├── exited: drain stdout/stderr
                              └── timeout: kill exact child, wait, fail test
```

超时固定五秒；kill 只针对刚 spawn 的 child handle，不按进程名批量终止。除 manifest 外还有专门测试覆盖 `-e/-l/--`、stdin、REPL、usage/runtime 分类和文件副作用。bytecode binary 也以相同策略覆盖四种 format、compile/runtime/usage error。

## Rust 知识映射与设计选择

- `Action` enum 保留 option 出现顺序；
- `Session` 封装可复用 VM，library convenience API 仍能一次性执行；
- `IsTerminal` 区分无参数 REPL 与管道脚本；
- `BufRead::lines` 与 source buffer 实现 continuation prompt；
- `BTreeSet` 让 CFG leaders 和 DOT 输出稳定排序；
- 手写 JSON escape/hex 避免为教学 viewer 引入序列化依赖，同时测试其 bytes 合同；
- `Command + try_wait + deadline` 把 hangs 变成确定性失败。

替代方案是把 REPL 每行写到临时文件并重新启动 binary，但那会丢失 globals/coroutine/package 状态。另一个方案是让 bytecode crate侵入 Compiler/VM；只读 Proto/trace event 保持工具与执行语义解耦。

## 从 v0.17 迁移了什么

- 新增：可复用 `Session`、宿主 arg table 注入和 action-ordered CLI；
- 重构：脚本 return value 不再污染 stdout；
- 新增：stdin、`-e/-l/-i/--`、help/version 和退出码合同；
- 新增：持久 REPL、表达式缩写、block 续行与错误恢复；
- 新增：`lua_bytecode` crate 和 text/JSON/CFG/trace binary；
- 新增：可开关 VM register trace events；
- 新增：fixture manifest、进程 timeout runner 和副作用验收；
- 保留：v0.17 host resources/modules 与此前全部语义测试；
- 未加入：binary chunk serializer/deserializer 或 C/native module loader。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v18-cli-tools/Cargo.toml -- `
  book/code/v18-cli-tools/examples/demo.lua
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace
```

示例在 Lua 内显式 print，因此 CLI stdout 是：

```text
1818
```

本快照共有 129 个测试：119 个 v0.17 既有测试、5 个 CLI/REPL process tests、3 个 bytecode library tests 和 2 个 bytecode binary process tests。所有子进程使用五秒 timeout，所有 workspace target 通过严格 Clippy。

## 本版本暂不支持

- REPL 没有 readline 历史、补全、TTY 编辑和 signal-aware Ctrl-C 恢复；
- multiline 判断依赖当前结构化编译错误类别，尚未覆盖所有 Lua pattern/string 边界；
- argv/path 在 Windows 仍采用 lossy UTF-8 adapter；
- JSON schema 还没有跨版本兼容承诺，v1.0 会冻结并登记版本；
- trace 是 instruction-before snapshot，没有 allocator bytes、GC phase transition 或 host callback event；
- process runner 只在 Rust tests 内使用，尚未成为通用 fixture CLI；
- Lua 5.1 binary chunk、C API 与 dynamic native module 仍分别属于 v1.1、v1.2、v1.3；
- v0.16/v0.17 已登记的 pattern、标准库和 debug 差异仍需 v1.0 收口。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>解释为什么函数单测不能证明 stdout/stderr/exit code、stdin EOF、文件副作用和 child
  timeout 合同。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为 REPL 增加 history abstraction，同时保留纯 BufRead backend；用非 TTY 进程测试证明
  history 不改变 continuation prompt 和 EOF 行为。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>构造一个不退出 fixture，观察 runner 是否在 deadline 后 kill 并 wait exact child；确认
  测试不会按进程名误杀，也不会遗留后台进程。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能从 argv 手推 Action 顺序与 arg table；
- 能区分 usage、compile/runtime 和正常退出合同；
- 能解释 JSON、CFG 与 trace 各自回答的问题；
- 能列出可靠子进程 runner 的完整生命周期。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 CLI contract tests、bytecode evidence、VM trace 和 fixture runner 是本版工具协议的扩展目标；
- [Bytecode](../ch03-compiler/bytecode.md) 可对照 raw instruction fields 与 JSON；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可解释 trace snapshot 中 base/top/register window；
- [用测试守住兼容性](../ch05-engineering/testing.md) 可对照进程级合同为何不能只用函数单测替代；
- 下一 checkpoint 是 v1.0：补齐登记差异，冻结能力矩阵，并以官方 Lua 5.1.5 与固定 C++ oracle 双差分收口。
