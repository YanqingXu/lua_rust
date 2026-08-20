# 下一站：闭包、GC 与协程

<div class="lesson-meta"><span>路线展望</span><span>高级主题</span><span>持续更新</span></div>

到这里，你已经走通 `Value → Lexer → AST → Bytecode → VM → Test`。这条最小主线足以帮助你独立阅读仓库，但 Lua 最有挑战的部分还在前方。

## 建议继续顺序

### 1. 函数调用与多返回值

先追踪参数如何进入新调用帧、`RETURN` 如何把结果写回调用者，再比较 `CALL` 的 B/C 字段怎样表达参数数量和返回数量。Lua 的“最后一个表达式可以展开多个结果”会让普通赋值也与调用协议相连。

源码入口：`lua_vm/src/execute.rs` 中的 `CALL`、`TAILCALL`、`RETURN`。

### 2. Closure 与 Upvalue

局部变量通常活在栈上，闭包却可能比创建它的函数活得更久。open upvalue 指向活跃栈槽，函数返回时必须 close 并把值转移到独立存储。

你会在这里再次遇到所有权，但问题从“谁拥有一个值”升级为“值的位置会随控制流改变”。

源码入口：`lua_core/src/upvalue.rs`、`lua_vm/src/execute.rs` 的 `CLOSURE` / `CLOSE`。

### 3. 垃圾回收与对象图

字符串、table、函数、userdata 和 thread 构成带环对象图。下一阶段可以从三类问题切入：

- 根在哪里：VM 栈、registry、全局表、活跃 Proto；
- 边如何追踪：每种 GC 对象暴露哪些子引用；
- 生命周期如何结束：mark、sweep、weak table 与 finalizer。

先阅读 [Runtime ownership RFC](https://github.com/YanqingXu/lua_rust/blob/main/docs/rust_migration/runtime_ownership_rfc.md)，再进入 `lua_core/src/gc/`。RFC 记录设计约束，代码展示当前落地状态，两者都不能代替真实测试证据。

### 4. Coroutine

Coroutine 把 VM 状态本身变成可以暂停和恢复的对象。除了独立栈与调用帧，还要处理 yield 边界、resume 参数、错误传播和协程祖先关系。

源码入口：`lua_core/src/thread.rs`、`lua_stdlib/src/coroutine.rs`。

### 5. 标准库与宿主边界

标准库是 Rust 与 Lua 互相调用的边界：参数从 `Value` 解码，Rust 执行宿主能力，再把结果编码回栈。IO、OS、路径和时间同时引入平台差异。

源码入口：`lua_stdlib/src/`。

## 把你的学习变成贡献

这本书与源码同仓维护。一次好的教程贡献可以是：

- 为真实实现添加一个稳定的 mdBook anchor，并在章节中引用；
- 把一个 migration report 提炼为面向读者的“语义 → Rust → 实现 → 练习”；
- 为现有章节补充能被自动运行的练习测试；
- 发现教程与代码不一致时，以代码和 oracle 证据为准修正文档。

<div class="checkpoint">
  <strong>完成当前主线</strong>
  <p>你现在应该能从一段 Lua 源码出发，指出它在仓库中经过的主要类型与模块，并为一次行为修改选择合适的测试层级。</p>
  <a href="../PREFACE.html">回到首页</a>
</div>
