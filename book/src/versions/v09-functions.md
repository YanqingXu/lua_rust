# v0.9：函数与调用帧

<div class="lesson-meta"><span>Checkpoint 10 / 20</span><span>嵌套 Proto</span><span>CallInfo</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>让函数值创建独立执行帧</strong>
  <p>前置：v0.8 的对象身份与寄存器 VM。新知识：Rc&lt;Proto&gt;、嵌套 Proto、调用窗口、
  显式 CallInfo、递归深度与 traceback。完成后你应能手推一次 CALL 的 caller/callee 状态。</p>
</div>

<a id="learning-predict"></a>
## 先预测再运行

为 `factorial(3)` 画三层 Lua frame。每层分别记录参数 `n`、返回 pc 和局部寄存器；不要
把它们画成同一组寄存器。再预测参数不足和参数过多时 callee 前 `num_params` 个 slot 的值。

## 本版本增量

v0.8 只有顶层 Proto 和一个寄存器窗口。v0.9 的唯一核心增量是“函数是值，调用创建新帧”：函数声明、`local function`、函数表达式、参数、单返回、递归、方法声明与冒号调用。

```lua
function factorial(n)
    if n <= 1 then return 1 end
    return n * factorial(n - 1)
end
```

<a id="learning-code-map"></a>
## 代码阅读地图

1. `Proto`：拥有不可变 code/constants/nested protos 与参数元数据；
2. `FunctionRef`：运行时身份，当前只共享 `Rc<Proto>`；
3. function AST/codegen：把函数体编译成 nested Proto；
4. call codegen：先求 callee/args，再复制到连续窗口；
5. `CALL A B C`：A 是函数和返回位置，B/C 描述参数；
6. `CallInfo`：记录 callee frame 及 caller source/line；
7. `VmError.traceback`：错误传播时复制 Lua 栈，而不是泄漏 Rust 栈细节；
8. 递归深度测试：在宿主 stack overflow 前返回结构化错误。

Function 和 Proto 分离是下一章 closure 的前提：同一段不可变代码可以创建多个运行时
函数对象，而每个函数未来还会携带不同 upvalue。`Rc<Proto>` 只共享代码，不代表两个
Function 身份相同。

## Function 与 Proto 不是同一概念

Proto 是不可变的编译产物：指令、常量、嵌套 Proto、参数数量、寄存器上限和 source/line metadata。Function 是运行时对象，当前用 `Rc<Proto>` 保持代码并按对象身份相等。同一 Function 的两个 Value 别名相等，由同一 Proto 创建的两个 Function 也是不同对象。

```rust
{{#include ../../code/v09-functions/crates/lua_core/src/lib.rs:function_ref}}
```

`CLOSURE A Bx` 从当前 Proto 的 nested-proto table 创建 Function 并写入 A。名字仍沿用 v0.6 的 local/global 解析；嵌套 Function 暂不捕获外层 local，Upvalue 将在 v0.11 成为 Function 的第二部分状态。

## 参数布局与 CALL

Codegen 先按 Lua 顺序求值 callee 和所有参数，再把它们复制到连续 call window。`CALL A B C` 的合同是：

- A：Function 寄存器，返回后也覆盖为结果；
- B：第一个参数寄存器；
- C：参数数量。

新帧创建 `max_stack_size` 个寄存器，前 `num_params` 个 slot 接收参数。参数不足时未写入 slot 保持 `nil`，多余参数暂时忽略。v0.10 会将这个固定宽度合同扩展为多返回与 vararg。

## 调用帧、base/top 与 traceback

每次调用拥有独立寄存器 Vec、pc 与 Proto。VM 的 `CallInfo` 额外记录 caller source/line 以及参数窗口的 base/top；运行时错误从最深帧向外传播时，会将当前 call stack 复制到 `VmError.traceback`。

不要从近 300 行 dispatch 中寻找调用协议，先按一次 CALL 的状态变化阅读：

```text
caller: evaluate callee and args into a contiguous window
  ↓ CALL
callee: allocate max_stack_size registers; copy num_params values
  ↓ execute until RETURN or VmError
caller: write result into CALL.A; restore caller pc
error: append caller source/line to traceback while unwinding
```

[查看完整 frame/dispatch 实现](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v09-functions/crates/lua_vm/src/lib.rs)

教学 VM 在 Rust 调用栈上递归执行帧，但所有 Lua 帧信息都显式存在 `CallInfo`。为了不让恶意递归先耗尽宿主栈，默认深度限制为 32；超出限制返回带完整 Lua traceback 的错误，而不是 Rust stack overflow。

## 递归与名字边界

全局函数在执行函数体时通过 `GETGLOBAL` 再次找到自身，因此可以递归。`local function f` 的语法和普通调用已支持，但“函数体捕获 local f 以递归”本质上是 Upvalue，将与其他 closure capture 一起在 v0.11 实现。本版本的递归验收因此使用全局声明，没有把未实现的捕获藏进 FunctionRef。

## 方法只是语法糖

```lua
function object:add(delta)
    return self.base + delta
end

object:add(2)
```

方法声明在参数列表前插入 `self`；冒号调用先求值 receiver 一次，读取 receiver["add"]，再把同一 receiver Value 插入第一个实参。测试用带计数副作用的 `get_object():add(2)` 证明 receiver 没有被求值两次。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v09-functions/Cargo.toml -- `
  --dump-bytecode book/code/v09-functions/examples/demo.lua
```

示例将 `factorial(5)` 传入 Table 方法，预期结果：

```text
type=number value=122
```

```powershell
cargo test --manifest-path book/code/v09-functions/Cargo.toml --workspace
```

验收覆盖全局递归、函数表达式、local function、无捕获嵌套函数、嵌套调用、参数补 nil/忽略多余参数、方法 self、receiver 单次求值、Function 身份、非函数调用、深度限制、source/line metadata 与多帧 traceback。

## 本版本暂不支持

- 多返回、值列表展开与 tail call；
- vararg 与 `...`；
- closure capture、Upvalue 与 local function 自捕获递归；
- Rust 宿主函数和标准库；
- 平坦化单 Vec 栈；当前每帧使用独立寄存器 Vec。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>手工跟踪 <code>factorial(3)</code> 的三个 CallInfo，记录参数 slot、pc、Proto 和返回值
  怎样回到 caller 的 A 寄存器。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>为二参数函数只传一个值，断言第二个 slot 为 nil；再构造三层类型错误并锁定每个
  traceback caller 行号。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>临时让冒号调用对 receiver 求值两次，运行单次求值测试，找出重复副作用发生在
  Codegen 的哪一步，再恢复临时寄存器复用。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能区分 AST function、Proto、FunctionRef 和 CallInfo；
- 能画出 CALL 前后的两个寄存器窗口；
- 能解释 Function 身份为什么不能直接等同于 Proto 指针；
- 能说明 Lua traceback 与 Rust 调用栈的边界。

## 从本版迁往 v0.10

从 v0.8 迁移时新增 `FunctionRef/Value::Function`、nested Proto、`CLOSURE/CALL`、函数 AST、参数 local、CallInfo 与 traceback。v0.10 会将单结果 CALL/RETURN 升级为可变宽度协议，并引入 vararg、tail call 和 generic for。
