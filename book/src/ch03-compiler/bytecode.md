# 06 · Bytecode：设计寄存器指令

<div class="lesson-meta"><span>预计 30 分钟</span><span>Rust：位布局 / 显式资源状态</span><span>编译器</span></div>

## 为什么不直接执行 AST

直接遍历 AST 可以做出解释器，但每次循环都要重复处理高层节点、名字解析和作用域判断。Lua 5.1 先把函数编译成紧凑字节码，VM 只执行少量固定格式的指令。

项目实现全部 38 个 Lua 5.1 opcode：

```rust
{{#include ../../../crates/lua_compiler/src/opcode.rs:lua_opcodes}}
```

注释里的 `R(A)` 表示寄存器 A，`K(Bx)` 表示常量表中的 Bx 项，`RK(C)` 表示 C 字段既可能引用寄存器，也可能引用常量。

## 从表达式到寄存器

考虑：

```lua
local answer = 40 + 2
return answer
```

概念上可以生成：

```text
LOADK   R(0) K(40)
LOADK   R(1) K(2)
ADD     R(0) R(0) R(1)
RETURN  R(0) 1
```

实际 codegen 还会折叠常量、复用寄存器并遵循 Lua 5.1 指令格式，但数据流已经很清楚：指令读取若干寄存器或常量，再写入目标寄存器。

## 一条指令的位布局

Lua 5.1 指令使用 32 位整数。最低 6 位存 opcode，其余字段按指令格式解释为 A/B/C、A/Bx 或 A/sBx。编码与解码使用掩码和位移，不依赖平台 struct 布局。

```text
31                 23 22              14 13       6 5       0
+--------------------+------------------+-----------+---------+
|        B (9)       |       C (9)      |   A (8)   | OP (6)  |
+--------------------+------------------+-----------+---------+
```

这里的 Rust 学习点不是记住位数，而是把二进制协议写成纯函数：相同输入总得到相同字段，边界可以用单元测试穷举。

## Codegen 也是资源管理

生成器需要持续回答：

- 哪个局部变量占用哪个寄存器？
- 临时表达式何时可以释放寄存器？
- `if` 与循环的跳转目标何时已知？
- 多返回值到底占几个位置？

项目把这些职责拆到 `reg_alloc`、`scope`、`jump`、`expr_emit` 与 `stmt_emit`。寄存器分配器不是操作系统级 allocator，而是一个小型状态机：申请临时编号、在表达式结束时归还、记录最大使用量。

跳转则使用“先占位、后回填”：生成 `JMP` 时目标可能未知，记录指令位置；作用域结束时计算偏移并 patch。Rust 的索引和显式错误检查让越界或无法表示的跳转更容易被定位。

## 看见自己的字节码

```powershell
cargo run -q -p lua_bytecode -- examples/arithmetic.lua
cargo run -q -p lua_bytecode -- examples/arithmetic.lua --format=json
```

文本输出适合阅读，JSON 输出适合差分工具。两种展示来自同一个 `Proto`，避免调试格式成为第二套事实来源。

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>预测 <code>local x=1; x=x+1; return x</code> 至少需要哪些 opcode，再查看实际输出。</li>
    <li>找到 <code>CREATE_ABC</code> 或等价编码函数，手算一次 <code>MOVE A=1 B=2</code>。</li>
    <li>比较 <code>if true then return 1 end</code> 与带变量条件版本的跳转结构。</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>Bytecode 把高层语义压成固定操作；codegen 的核心工作是管理寄存器、常量、作用域和尚未确定目标的跳转。</p>
</div>

继续探索：[`opcode.rs`](https://github.com/YanqingXu/lua_rust/blob/main/crates/lua_compiler/src/opcode.rs) · [`codegen/`](https://github.com/YanqingXu/lua_rust/tree/main/crates/lua_compiler/src/codegen)
