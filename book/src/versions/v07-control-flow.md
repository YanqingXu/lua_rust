# v0.7：控制流

<div class="lesson-meta"><span>Checkpoint 8 / 20</span><span>跳转回填</span><span>循环上下文</span></div>

## 本版本增量

v0.6 的 statement 只能顺序执行。v0.7 的唯一核心增量是将结构化控制流降级为 program counter 的改变：`if/elseif/else`、`while`、`repeat until`、numeric `for`、`break` 和嵌套 block。

变量、赋值与表达式语义不改变；它们作为回归测试继续运行。

## 从结构化语句到 pc

| Lua 结构 | 字节码形状 |
|---|---|
| `if c then a else b end` | condition；false jump 到 b；a 后 jump 到 end |
| `while c do body end` | condition；false jump 到 end；body；back edge 到 condition |
| `repeat body until c` | body；condition；false back edge 到 body |
| `break` | 占位 `JUMP`，循环结束后回填 |
| numeric `for` | `FORPREP`；body；`FORLOOP` back edge |

前向目标在生成 jump 时尚不存在。Codegen 先写入 offset=0 的 AsBx 指令并记录 pc，编译完目标 block 后计算 `target - jump_pc - 1`。偏移超出 18 位 AsBx 范围时返回 `CompileError`。

```rust
{{#include ../../code/v07-control-flow/crates/lua_compiler/src/lib.rs:control_flow_codegen}}
```

## 循环上下文与 break

Codegen 使用 `Vec<Vec<jump_pc>>` 表示嵌套循环。进入循环时 push 一份 break patch list，遇到 `break` 就把占位 jump 加入最后一层，离开循环时统一回填。因此嵌套循环的 break 天然指向最近循环；栈为空时的 break 是结构化编译错误。

当前 local 只是寄存器，退出作用域无需运行时指令。到 v0.11 出现 open upvalue 后，break/back edge 还必须发出 `CLOSE`，本章的 loop context 将成为那次迁移的附着点。

## numeric for 的寄存器窗口

numeric for 预留连续四个寄存器：

```text
A     internal index
A+1   limit
A+2   step
A+3   body 可见的 loop variable
```

initial/limit/step 只求值一次。`FORPREP` 验证三者是数字，将 internal index 设为 `initial-step` 并跳到 `FORLOOP`；`FORLOOP` 先加 step，再根据 step 正负选择 `<= limit` 或 `>= limit`。继续时同步 A 与 A+3 并跳回 body。

```rust
{{#include ../../code/v07-control-flow/crates/lua_vm/src/lib.rs:loop_vm}}
```

本教学版将 step=0 和 NaN 定义为结构化运行时错误，避免无进展循环；该边界已在测试与 deviation 说明中显式记录，不会静默近似官方实现。

## while 与 repeat 的差别

`while` 在 body 前求值条件，所以 body 可能一次也不执行。`repeat` 在 body 后求值，至少执行一次；而且 repeat body 定义的 local 在 `until condition` 中仍可见。因此 repeat 的 scope 要包含 body 与 condition，不能在编译 condition 前弹出。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v07-control-flow/Cargo.toml -- `
  --dump-bytecode book/code/v07-control-flow/examples/demo.lua
```

示例在 numeric for 中组合 if/elseif/else 与 break，预期结果：

```text
type=number value=306
```

```powershell
cargo test --manifest-path book/code/v07-control-flow/Cargo.toml --workspace
```

验收覆盖分支唯一性、while/repeat 条件位置、repeat local 可见性、正/负 step、最近循环 break、step=0、循环外 break 以及 v0.6 全部变量回归。

## 本版本暂不支持

- generic for 与迭代器调用协议；
- Table、函数、调用帧和 closure；
- goto/label；Lua 5.1 本身也不提供该语法；
- 被 upvalue 捕获 local 的运行时关闭。

## 练习与迁移

1. 手工计算示例中 `FORLOOP` 的负 offset 和 break 的正 offset。
2. 为 `while false` 和 `repeat ... until true` 分别记录 body 执行次数。
3. 构造两层 while，证明内层 break 不会终止外层。
4. 将 repeat scope 提前弹出，观察 local-in-condition 测试如何失败。

从 v0.6 迁移时新增控制流 AST、`JUMP/FORPREP/FORLOOP`、前向/后向回填与 loop context stack。v0.8 将保留这套 pc 模型，新增第一种共享可变对象 Table。
