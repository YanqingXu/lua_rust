# v0.6：变量、赋值与作用域

<div class="lesson-meta"><span>Checkpoint 7 / 20</span><span>statement AST</span><span>作用域栈</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>让名字绑定到可更新的存储位置</strong>
  <p>前置：v0.5 的寄存器与 Proto。新知识：符号表、作用域栈、局部寄存器生命期、
  HashMap owner、并行赋值。完成后你应能区分编译期 local 解析和运行时 global 查找。</p>
</div>

<a id="learning-predict"></a>
## 先预测再运行

手推 `local a,b=1,2; a,b=b,a; return a*10+b` 的每次读写。右值必须全部读取完才开始
写左值，所以结果是 21。再预测 `local x=1; do local x=x+1 end; return x`：initializer
读取外层 x，内层 x 离开 block 后消失，结果仍为 1。

## 本版本增量

v0.5 的 VM 只能执行一个无名字绑定的表达式。v0.6 的唯一核心增量是“脚本可以保存与更新状态”：Parser 新增 statement AST，Codegen 新增 global/local 名字解析、普通/多重赋值与 `do ... end` 作用域，VM 新增全局表。

从这一版开始，输入是 Lua chunk 而不是裸表达式：

```lua
local left, right = 10, 20
left, right = right, left
return left * 100 + right
```

## Lua 名字语义

- 未定义 global 的值是 `nil`，读取它不是运行时错误；
- 没有 initializer 的 local 以 `nil` 初始化；
- local 的 initializer 在新 local 可见之前求值，所以 `local x = x` 读取外层 `x`；
- 内层 block 可以遮蔽同名 local，离开 block 后恢复外层绑定；
- 多重赋值先求完全部右值，再从左到右写入，因此 `a,b=b,a` 能交换值；
- 右值不足时补 `nil`，多余右值仍会求值但不会写入。

<a id="learning-code-map"></a>
## 代码阅读地图

1. statement AST：区分声明、赋值、block 和 return；
2. `Scope`：拥有当前层的 `HashMap<ByteString, u8>` 与 register base；
3. local lookup：从内层向外层查找，找到后生成 `MOVE`；
4. initializer codegen：先在旧 scope 求值，再登记新 local；
5. assignment codegen：先缓存所有右值和复合左值地址，再统一写入；
6. `GETGLOBAL/SETGLOBAL`：用常量表中的字节名字访问 VM 状态；
7. 交换、遮蔽、寄存器上限测试：锁定三条关键不变量。

Rust 所有权在这里划分两个世界：Codegen 拥有 `Scope` 和名字 key，编译完成后这些符号表
即可释放；VM 拥有 global table，能跨多次 execute 保留值。不要让 Proto 拥有 globals，
否则不可变程序与可变运行状态会混在一起。

## local 为什么是寄存器

局部名字在编译时就能解析，因此不需要运行时 HashMap 查找。每个活跃 local 占据一个稳定寄存器，临时表达式寄存器从 local 区间之后开始。读取 local 时先 `MOVE` 到临时寄存器，避免一元/二元指令覆盖变量本身。

作用域栈的每层记录进入时的 register base 与名字到寄存器的映射。离开 block 时弹出该层并恢复 base，内层 local 占据的寄存器就可以被下一个 block 复用。

先用伪代码抓住 scope 的 owner 与寄存器恢复，不必一次阅读完整 binder：

```text
enter_scope:  保存当前 next_register，push 空名字表
declare:      当前名字表[name] = next_register；next_register += 1
resolve:      从 scopes 最后一个向前查找
leave_scope:  pop 名字表；next_register = 进入时保存的 base
```

[查看本版完整 binder 源码](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v06-variables-scope/crates/lua_compiler/src/lib.rs)

这里不使用存储 `&str` 的符号表，因为 Lua 名字和字符串的底层输入仍然是字节。`HashMap<ByteString, u8>` 拥有 key，让 Lexer 的输入借用不会泄漏进 Codegen 生命周期。

## 并行赋值不是逐个赋值

Codegen 为所有右值分配临时寄存器，完整编译后才解析并写入左值。如果把求值与写入放在同一循环，`a,b=b,a` 在读取第二个右值时已经丢失旧 `a`。这个测试是赋值实现的核心证据。

local declaration 还有一个额外顺序要求：先在旧 scope 中编译全部 initializer，再把新名字插入当前 scope。

## global 为什么在 VM 中

global 是运行时状态，同一个 `Vm` 可以在多次 execute 之间保留它。`GETGLOBAL` 对不存在的 key 返回 `nil`；`SETGLOBAL` 写入 `nil` 时删除 key。指令使用 ABx 格式：A 是值寄存器，Bx 指向保存名字的字符串常量。

```rust
{{#include ../../code/v06-variables-scope/crates/lua_vm/src/lib.rs:globals}}
```

可选方案是把 global table 放在 `Proto`。那会混淆“不可变编译产物”与“每次执行的可变状态”，也使同一 Proto 无法在不同环境中运行。因此 global 明确属于 VM/Runtime。

## 运行与预期结果

```powershell
cargo run --manifest-path book/code/v06-variables-scope/Cargo.toml -- `
  --dump-bytecode book/code/v06-variables-scope/examples/demo.lua
```

反汇编中可以看到交换前先读入两个临时寄存器，也可以看到 block 内的 global 写入：

```text
0004 [  2] Move         2 1
0005 [  2] Move         3 0
0006 [  2] Move         0 2
0007 [  2] Move         1 3
0011 [  6] SetGlobal    3 3
type=number value=2010
```

## 测试与验收

```powershell
cargo test --manifest-path book/code/v06-variables-scope/Cargo.toml --workspace
```

验收覆盖：

- `a,b=b,a` 使用赋值前值；
- block 内层遮蔽不改变外层 local；
- initializer 中的同名名字解析为外层绑定；
- 未定义 global 与未初始化 local 均为 `nil`；
- global/local 使用 `GETGLOBAL/SETGLOBAL` 与 `MOVE` 两套存储；
- 第 256 个活跃寄存器产生结构化 `CompileError` 而不是整数溢出或 panic。

## 本版本暂不支持

- `if`、`while`、`repeat`、`for` 和 `break`；
- Table field 左值与其他复合赋值目标；
- 函数、调用和闭包；
- 多返回展开；`return a,b` 的完整宽度规则到 v0.10 实现。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>解释为什么 local 可以编译成寄存器，而 global 必须在 VM 中按名字查找；指出两者
  各自的 owner 和生命期。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为 <code>local x=x</code> 的“有外层 local”和“只有 global”两种情况写测试，并为
  <code>local a,b,c=1</code> 断言 nil 补齐。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>临时把赋值改成“求一个写一个”，运行交换测试；记录旧值在哪次写入后丢失，再恢复
  右值缓冲协议。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能画出嵌套作用域栈和每个 local 的寄存器；
- 能解释声明 initializer 为什么在新名字可见前求值；
- 能手推并行赋值的“先读后写”顺序；
- 能区分 Proto 的不可变数据与 VM 的可变 global 状态。

## 从本版迁往 v0.7

从 v0.5 迁移时，裸表达式入口升级为 chunk/statement AST，新增 `Scope`、local binder、赋值右值缓冲、`GETGLOBAL/SETGLOBAL` 和 VM global table。v0.7 会保留这套名字与寄存器合同，再用跳转回填加入条件和循环。
