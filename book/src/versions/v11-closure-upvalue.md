# v0.11 · Closure 与 Upvalue

<div class="lesson-meta"><span>核心增量：跨函数共享局部状态</span><span>workspace</span><span>checkpoint v0.11</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>把捕获的位置与当前位置的值分开</strong>
  <p>前置：v0.10 的 frame、寄存器窗口和尾调用。新知识：词法 resolver、共享稳定 slot、
  open/closed 状态、Rc&lt;RefCell&gt; 身份迁移。完成后你应能跟踪一个 local 从 stack 到
  closed value 的全过程。</p>
</div>

v0.10 已有函数对象、多返回和尾调用，但嵌套函数遇到外层 local 时仍把名字当作 global。本版增加一个核心能力：**函数可以捕获定义位置可见的 local，并在原调用帧结束后继续访问它**。

<a id="learning-predict"></a>
## 先预测再运行

画两个 closure 同时捕获 `current` 的对象图。第一次 closure 修改后，第二个必须看到新值，
所以捕获时复制 Value 是错误答案。再画 block 退出后原寄存器被新 local 复用的情况：若
upvalue 永远指向 slot，旧 closure 会被新 local 污染，因此退出时必须 close。

## 本版完成后能做什么

```lua
function make_counter(start)
    local current = start
    return function(step)
        current = current + step
        return current
    end
end

local counter = make_counter(10)
counter(2) -- 12
counter(3) -- 15
```

还包括 Lua 5.1 closure 语义中容易遗漏的边界：

- 两个兄弟 closure 捕获同一个 local 时共享同一个 upvalue；
- `local function factorial` 可以通过 upvalue 捕获自身；
- 孙函数捕获祖先 local 时，中间函数会转发 upvalue；
- 块退出、`break`、普通 return 和 tail call 都会关闭对应 upvalue；
- numeric/generic for 每轮关闭捕获的循环变量；
- 同一 Proto 每次执行 `CLOSURE` 都产生身份不同的函数对象。

<a id="learning-code-map"></a>
## 代码阅读地图

1. resolver：local → parent local → parent upvalue → global；
2. `UpvalueDescriptor`：记录从 parent stack 还是 parent upvalue 绑定；
3. `Closure`：组合共享 Proto、绑定后的 UpvalueRef 与独立身份；
4. `StackSlot`：给活跃寄存器提供稳定共享地址；
5. open-upvalue map：以 `(frame_id, register)` 去重兄弟捕获；
6. `GETUPVAL/SETUPVAL/CLOSE`：读取、修改和迁移状态；
7. block/break/for/return/tail-call 清理点：决定何时 close；
8. 兄弟共享、寄存器复用、跨两层捕获测试：证明模型完整。

阅读时始终区分三种身份：Proto 是代码模板；Closure 是一次运行时函数对象；UpvalueRef
是可被多个 Closure 共享的变量单元。把其中任意两种合并，都会在身份或共享测试中失败。

## Proto 不是 Closure

Proto 是不可变的函数模板：字节码、常量、参数和 upvalue 描述符。Closure 才是运行时对象，它组合一个 Proto 和一组已经绑定的 `UpvalueRef`。

```text
Proto ──描述──> upvalue 0: parent stack R[2]
  │             upvalue 1: parent upvalue[0]
  │
  └─ CLOSURE ──> FunctionRef { proto, bound_upvalues }
```

因此两个 closure 可以共享 Proto，却必须拥有独立函数身份；它们也可以让某些 upvalue 指向同一个共享单元。

三个运行时对象的关系可以先写成类型草图：

```text
Closure { proto: Rc<Proto>, upvalues: Vec<UpvalueRef> }
UpvalueRef → RefCell<Open(StackSlot) | Closed(Value)>
FunctionRef → Rc<Closure>  // identity belongs here
```

[查看 FunctionRef/UpvalueRef 完整实现](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v11-closure-upvalue/crates/lua_core/src/lib.rs)

## 编译器怎样解析跨函数名字

每个 Codegen 有一个 `Resolver`：保存当前函数的块级 local、已经分配的 upvalue 描述符，以及父函数 resolver。

Resolver 的递归只返回两种捕获路径：父 frame 的寄存器，或父 closure 已有的 upvalue。
中间函数即使不直接读取该名字，也必须把第二种路径保存在自己的 descriptor 中。
[查看完整词法 resolver](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v11-closure-upvalue/crates/lua_compiler/src/lib.rs)

名字解析顺序固定为：

1. 当前函数从内到外查找 local；
2. 请求父函数为子函数提供捕获描述符；
3. 父函数若找到自己的 local，返回 `(in_stack=true, register)`；
4. 父函数若也要向上捕获，先建立自己的 upvalue，再返回 `(in_stack=false, upvalue_index)`；
5. 所有词法层级都没有找到时才解析为 global。

这一步解释了为什么孙函数捕获祖先 local 时，中间 closure 即使不读取该名字，也必须持有并转发它。超过 255 个 upvalue 会得到结构化编译错误，不能静默退化为 global。

`local function f()` 需要特殊处理：先声明 local `f`，再编译函数体并赋值。普通 `local f = function()` 则先求右值，右侧看到的是声明前的外层绑定。这一区别直接决定局部递归是否正确。

## Open 与 Closed Upvalue

活跃调用帧中的 upvalue 不能立刻复制值，否则兄弟 closure 看不到彼此的修改。本版使用两个状态：

```text
Open(StackSlot) ── CLOSE ──> Closed(Value)
      │                           │
      └── 与 VM 寄存器同一单元     └── 与已复用的寄存器脱离
```

VM 用 `(frame_id, register)` 作为 open-upvalue 表的 key。第一次捕获创建 `UpvalueRef`，后续兄弟 closure 得到同一个引用。关闭时复制当前值、从 open 表移除，并让下一次复用该寄存器时创建新的 upvalue。

```rust
{{#include ../../code/v11-closure-upvalue/crates/lua_vm/src/lib.rs:open_upvalues}}
```

寄存器因此从 `Vec<Value>` 迁移为稳定地址的 `Vec<Rc<RefCell<Value>>>`。这不是最终 GC 设计，而是用最小机制清楚展示“位置”和“值”必须分离。

## CLOSE 应该出现在哪里

- 离开任意词法块：关闭该块起始寄存器及以上的 upvalue；
- `break`：先关闭从循环块开始的所有捕获，再跳到循环出口；
- numeric/generic for：每轮 body 结束都关闭可见循环变量，使三次 closure 分别得到 1、2、3；
- return/error/tail call：执行帧的统一清理路径关闭该 frame 的全部 open upvalue。

只在函数 return 时关闭是不够的。块结束后寄存器可以立刻复用；若 upvalue 仍指向原槽，后续 `local` 会悄悄改写已经逃逸的 closure。

## Rust 知识映射与设计选择

- `Rc<RefCell<Value>>` 提供稳定栈槽和受控内部可变性；
- `Rc<RefCell<UpvalueState>>` 让多个 closure 共享 open/closed 状态；
- `HashMap<(frame_id, register), UpvalueRef>` 保证同一位置只创建一个 open upvalue；
- `Rc<Closure>` 而不是 `Rc<Proto>` 给每次运行时 closure 创建独立身份；
- resolver 的父指针表达词法层级，upvalue descriptor 表达运行时绑定路径。

替代方案之一是把捕获值在 `CLOSURE` 时直接复制进函数对象；计数器和兄弟 closure 测试会失败。另一个方案是让 upvalue永久指向栈槽；块退出后的寄存器复用测试会失败。最终实现会让 GC heap 管理 closure/upvalue，而不是继续叠加 `Rc`。

## 从 v0.10 迁移了什么

- 删除：`FunctionRef(Rc<Proto>)` 这个“函数等于模板”的模型；
- 重构：函数对象变为 `Rc<Closure>`，包含 Proto 与绑定后的 upvalue 列表；
- 重构：寄存器槽拥有稳定共享地址，值读写经由内部可变性完成；
- 新增：Proto upvalue descriptors、词法 resolver 与祖先捕获转发；
- 新增：`GETUPVAL`、`SETUPVAL`、`CLOSE`；
- 新增：执行帧 ID、open-upvalue 表和无论成功/错误都执行的统一关闭路径；
- 修正：`local function` 的声明顺序、每轮 for 捕获和 break 清理。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v11-closure-upvalue/Cargo.toml -- `
  --dump-bytecode book/code/v11-closure-upvalue/examples/demo.lua
cargo test --manifest-path book/code/v11-closure-upvalue/Cargo.toml --workspace
```

示例最终输出应为：

```text
type=number value=135123
```

新增测试覆盖计数器、兄弟共享、跨两层捕获、局部递归、块/break 关闭、两类 for 捕获、closure 身份和 tail-call 清理。对应编译器测试还检查 stack/parent descriptor 以及三个 upvalue opcode。

## 本版本暂不支持

- 没有 metatable 和元方法；closure 不能通过 `__call` 间接调用；
- closure、table 和 upvalue 仍由 `Rc` 强引用，局部递归可能形成无法回收的环；
- 没有完整 GC、弱引用、finalizer 和 userdata；
- 没有 coroutine，不能跨 yield 保存 open upvalue；
- 内建迭代器仍是 v0.10 的临时 `NativeFunction` 枚举。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>解释 <code>local f=function() return f end</code> 与
  <code>local function f() return f end</code> 的声明顺序，以及 resolver 会捕获哪个绑定。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>实现“递增”和“重置”两个 closure 证明共享 upvalue；再写三层捕获测试，检查
  descriptor 的 <code>in_stack</code> 差异。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>删除 for body 末尾的 CLOSE，观察三个 closure 是否共享最后一次循环 slot；用对象图
  解释失败后恢复每轮关闭。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 能手推跨两层名字解析和 descriptor 链；
- 能解释兄弟 closure 为什么共享 UpvalueRef 而不是复制 Value；
- 能列出所有必须执行 CLOSE 的控制流出口；
- 能说明 open → closed 后寄存器为什么可以安全复用。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 closure/upvalue 对象图是本版模型的最终映射目标；
- [用 enum 表示 Lua Value](../ch02-runtime/value.md) 可复习函数值如何进入统一 Value；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 可对照新增三个 opcode 的 dispatch；
- 下一 checkpoint：[v0.12 · Metatable 与元方法](v12-metatable.md)。
