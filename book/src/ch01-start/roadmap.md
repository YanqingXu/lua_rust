# 学习路线

<div class="lesson-meta"><span>预计 5 分钟</span><span>路线导览</span><span>无需前置知识</span></div>

我们不会按仓库目录逐文件阅读，而是顺着解释器的数据流前进。每一站都解决一个可以被运行或测试的问题。

## 主线地图

| 阶段 | 要回答的问题 | Rust 学习点 | 代码入口 |
|---|---|---|---|
| Value | Rust 如何表示运行时才知道类型的值？ | `enum`、`match`、`Option` | `lua_core/value.rs` |
| ByteString | 为什么 Lua 字符串不能直接用 `String`？ | 字节切片、所有权、UTF-8 边界 | `lua_core/byte_string.rs` |
| Table | 一个容器怎样同时像数组和字典？ | `Vec`、`HashMap`、可变借用 | `lua_core/table.rs` |
| Lexer | 怎样把源码切成可处理的单元？ | 生命周期、切片、状态机 | `lua_compiler/lexer.rs` |
| Parser | 怎样在 Rust 中表示递归语法？ | `Box<T>`、递归 enum、`Result` | `lua_compiler/ast/`、`parser/` |
| Bytecode | 怎样把结构化语法压成 VM 指令？ | 位运算、显式状态、资源管理 | `lua_compiler/codegen/` |
| VM | 指令如何真正改变程序状态？ | 借用边界、错误传播、dispatch | `lua_vm/execute.rs` |
| Testing | “能跑”与“兼容”有什么区别？ | 集成测试、差分测试、CI | `tests/`、`tools/` |

## 建议的阅读节奏

第一次阅读时，每章只做三个动作：

1. 运行章节开头的 Lua 例子，先得到直觉；
2. 阅读被引用的核心实现，尝试用自己的话解释数据结构；
3. 完成章末检查点，不必立即深入所有源码链接。

第二遍再沿着“继续探索”进入测试、RFC 和完整模块。这样能避免在 GC、元方法或多返回值的细节里过早迷路。

## 里程碑

- **里程碑 A：读懂值。** 能说明 `Value::String` 为什么保存 GC 引用，而 `Value::Number` 直接保存 `f64`。
- **里程碑 B：读懂编译。** 能手工解释 `local answer = 40 + 2` 会出现哪些 Token 和 AST 节点。
- **里程碑 C：读懂执行。** 能用寄存器变化解释 `LOADK`、`ADD`、`RETURN`。
- **里程碑 D：读懂证据。** 能区分单元测试通过、fixture 一致和 Lua 5.1 完整兼容。

<div class="checkpoint">
  <strong>路线检查</strong>
  <p>如果你的主要目标是学习 Rust，优先读 Value、ByteString、Table 和 Parser；如果主要目标是解释器，按目录顺序完整阅读。</p>
</div>
