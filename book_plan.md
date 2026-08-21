# 《从 Lua 解释器开始学习 Rust》版本化教学总规划

> 状态：v0.0–v1.0 教学快照已实施；v1.1+ 待实施<br>
> 目标读者：掌握 Rust 基础语法、希望通过真实项目进入解释器与运行时工程的初中级开发者<br>
> 最终目标：从最小命令行骨架逐步实现可验证的 Lua 5.1 语言运行时

## 1. 文档目标

当前 `book/` 已经能够围绕 Value、ByteString、Table、Lexer、Parser、Bytecode、VM
和兼容性测试讲解成熟实现，但它更接近“专题阅读指南”。版本化教学主线要解决另一
个问题：让读者从一份可以完全理解的最小代码出发，每次只引入一个核心能力，最终亲手
走到完整的 Lua 5.1 语言运行时。

这条路线必须同时满足以下要求：

- 每个版本都是独立 checkpoint，可以单独编译、运行和测试；
- 每个版本只引入少量新语义，并明确说明为什么现在需要它；
- Rust 知识随着解释器问题自然出现，而不是脱离上下文罗列语法；
- 教学实现与生产实现概念一致，但允许早期版本使用更容易理解的结构；
- 每个临时设计都必须登记限制，并在后续版本中明确迁移或删除；
- “能运行示例”与“兼容 Lua 5.1”分开表述，兼容结论必须有 oracle 证据；
- 当前专题章节继续保留，用作版本主线之外的深入阅读材料。

## 2. 最终范围

v1.0 的“完整 Lua 解释器”在本书中定义为 **Lua 5.1 语言运行时**，包括：

- Lua 5.1 源码的词法、语法、AST、字节码生成和寄存器 VM；
- 动态值、任意字节字符串、Table、函数、闭包、Upvalue 和 Metatable；
- 多返回、vararg、尾调用、迭代器、错误处理和 Coroutine；
- 真实 GC、弱表、Finalizer、增量回收和运行时生命周期；
- base、math、string、table、coroutine、io、os、package、debug 标准库；
- Lua 文件模块、CLI、REPL、字节码查看器、fixture 与差分测试；
- 对官方 Lua 5.1.5 与固定 C++ oracle 的可复现兼容性证据。

以下能力不属于 v1.0 语言运行时，进入 v1.1 之后的进阶路线：

- Lua 5.1 binary chunk 序列化、反序列化与 verifier；
- 稳定 C API/ABI、公开 headers、lauxlib 和 C consumer；
- `package.loadlib` 与动态原生模块；
- 生产沙箱、资源配额、跨平台 SDK 和系统级性能治理。

## 3. 快照目录与执行合同

### 3.1 统一目录

每个版本保存在独立目录中：

```text
book/code/vXX-name/
├── Cargo.toml
├── Cargo.lock
├── src/                    # 单 crate 阶段
├── crates/                 # workspace 阶段
├── tests/
├── examples/
│   └── demo.lua
└── README.md               # 版本能力、命令和已知限制
```

版本目录使用补零编号，确保文件浏览器中的顺序与课程顺序一致：

```text
v00-skeleton
v01-value
...
v18-cli-tools
v100-lua51-runtime
```

v0.0–v0.4 使用单 crate。每个 `Cargo.toml` 包含空 `[workspace]`，避免被仓库根
workspace 隐式吸收。v0.5 开始使用独立 workspace，并设置 `lua_app` 为默认成员。
v0.16 增加 `lua_stdlib`，v0.18 增加 `lua_bytecode`。

### 3.2 统一运行方式

进入任一快照后，读者都可以使用相同命令：

```powershell
Set-Location book/code/vXX-name
cargo run -- examples/demo.lua
cargo test
```

从仓库根目录执行时使用完整路径：

```powershell
cargo run --manifest-path book/code/vXX-name/Cargo.toml -- `
  book/code/vXX-name/examples/demo.lua
cargo test --manifest-path book/code/vXX-name/Cargo.toml
```

workspace 快照的 `default-members` 必须保证上述命令无需读者额外选择 package。

### 3.3 每个版本的章节模板

每一个版本的教学章节都必须按相同顺序编写：

1. **上一版回顾**：上一版可以运行什么、还缺什么；
2. **本版目标**：本版唯一核心增量和最终可观察结果；
3. **Lua 语义**：先用 Lua 程序定义行为；
4. **实现难点**：把语言现象拆成状态、数据和边界问题；
5. **Rust 知识**：只解释解决当前问题所需的 Rust 能力；
6. **设计与替代方案**：说明选择、代价和未来迁移点；
7. **代码导读**：通过 mdBook anchor 引用本版本真实源码；
8. **从上一版迁移**：列出新增、修改和删除的关键结构；
9. **运行实验**：给出命令、输入和确定的预期输出；
10. **自动测试**：列出单元、集成和必要的差分测试；
11. **练习**：至少包含理解题、实现题和调试题；
12. **暂不支持**：明确禁止把后续版本能力伪装成本版能力；
13. **完成检查点**：读者能够用自己的话解释的核心不变量；
14. **继续深入**：链接到当前成熟实现和专题章节。

### 3.4 快照质量合同

每个版本合入前必须满足：

- `cargo fmt --check`、`cargo clippy -- -D warnings` 和 `cargo test` 通过；
- `examples/demo.lua` 能展示本版新增能力；
- 错误场景同时断言 stderr 与非零退出码；
- 本版没有依赖后续快照的相对路径或构建产物；
- 所有公开行为都有至少一个成功测试和一个失败/边界测试；
- `README.md` 与章节中的“暂不支持”保持一致；
- 被文章引用的代码使用稳定 mdBook anchor，不依赖脆弱行号；
- 与官方 Lua 的比较必须注明是示例、代表性差分还是完整验收。

## 4. 版本总览

| 版本 | 教学里程碑 | 主要 Rust 知识 | 架构形态 |
|---|---|---|---|
| v0.0 | 最小解释器骨架 | Cargo、模块、Result、IO | 单 crate |
| v0.1 | Value 与字面量 | enum、match、字节所有权 | 单 crate |
| v0.2 | Lexer 与 Token | 切片、生命周期、状态机 | 单 crate |
| v0.3 | Parser 与最小 AST | Box、递归类型、结构化错误 | 单 crate |
| v0.4 | 完整表达式语义 | 运算分发、短路、错误传播 | 单 crate |
| v0.5 | Bytecode 与寄存器 VM | workspace、位运算、dispatch | 4 crate workspace |
| v0.6 | 变量、赋值与作用域 | 符号表、借用、资源范围 | 4 crate workspace |
| v0.7 | 控制流 | 跳转回填、循环上下文 | 4 crate workspace |
| v0.8 | Table | Vec、HashMap、Rc、RefCell | 4 crate workspace |
| v0.9 | 函数与调用帧 | 对象身份、栈窗口、递归 | 4 crate workspace |
| v0.10 | 多返回、vararg 与迭代器 | 可变结果、栈区间 | 4 crate workspace |
| v0.11 | Closure 与 Upvalue | 共享状态、位置迁移 | 4 crate workspace |
| v0.12 | Metatable 与元方法 | 动态分发、递归保护 | 4 crate workspace |
| v0.13 | Runtime 所有权与完整 GC | 对象图、句柄、unsafe 边界 | Runtime + Heap |
| v0.14 | 增量 GC、弱表与 Finalizer | 状态机、不变量、重入 | Runtime + 增量 GC |
| v0.15 | Coroutine | 暂停/恢复、跨状态调度 | 多 LuaState Runtime |
| v0.16 | 核心标准库 | 宿主函数适配、API 合同 | 增加 lua_stdlib |
| v0.17 | 宿主标准库与 Lua 模块 | 资源生命周期、平台抽象 | 完整运行时 workspace |
| v0.18 | CLI、REPL 与字节码工具 | 终端、进程、结构化输出 | 增加 lua_bytecode |
| v1.0 | Lua 5.1 兼容收口 | 双 oracle、发布治理 | 教学线与生产线对齐 |

## 5. 各版本详细规划

### v0.0：最小解释器骨架

**快照目录**：`book/code/v00-skeleton/`

**上一版状态**：无代码。读者只需要已经安装 Rust 工具链。

**本版目标**：建立可重复的“输入 → 执行器 → stdout/stderr/exit code”链路。这个
版本故意不解析 Lua，避免把 CLI、文件 IO 和语言实现混在第一次练习中。

**Lua/程序能力**：

- 接收一个脚本路径或从 stdin 读取字节；
- 为源码记录 chunk name；
- 把输入交给 `Interpreter::run` 占位入口；
- 成功时打印已读取的字节数，失败时输出带来源的错误；
- 统一返回成功、用户输入错误和 IO 错误退出码。

**Rust 学习点**：Cargo package、`main`/`lib` 分离、`std::env::args_os`、
`std::fs::read`、`Result`、`?`、错误展示与进程退出。

**主要代码**：

- `SourceChunk { name, bytes }` 负责保存输入；
- `CliError` 区分参数和 IO 错误；
- `Interpreter::run(&SourceChunk)` 是以后保持稳定的执行入口。

**章节安排**：项目初始化；为什么源码入口是 bytes；错误为什么不是 panic；第一条
进程级测试。

**运行实验**：读取 `examples/demo.lua`，stdout 显示 chunk name 和字节数。

**测试与验收**：

- 文件输入与 stdin 输入均能到达执行器；
- 不存在文件时 stderr 包含路径，退出码非零；
- 空输入可被读取，不因索引访问 panic；
- `cargo test` 和一次真实进程 smoke test 通过。

**暂不支持**：任何 Token、Lua 值、语法或执行语义。

### v0.1：Lua Value 与字面量

**快照目录**：`book/code/v01-value/`

**上一版状态**：只能读取源码，不能理解输入。

**本版目标**：让解释器识别并展示一个 Lua 字面量，同时建立后续所有阶段共用的
动态值模型。

**Lua 语义**：

- 支持 `nil`、`true`、`false`、十进制 number 和带引号 byte string；
- 只有 `nil` 和 `false` 为假；
- number 使用 `f64`；
- string 是任意字节，不等同于 Rust `String`；
- 基础相等只比较同类立即值或字符串内容。

**Rust 学习点**：`enum Value`、穷尽 `match`、`Option`、`Box<[u8]>`、借用、
`PartialEq`、显式 lossy display。

**主要代码变化**：新增 `ByteString`、`Value`、`ValueType`、字面量读取器和
`display_value`；执行器返回 `Value`，CLI 只负责显示。

**章节安排**：静态 Rust 如何表示动态 Lua；字节字符串；真值；安全访问器与失败策略。

**运行实验**：分别执行 `42`、`false`、`"Lua"` 和含 NUL 的字符串输入。

**测试与验收**：

- 每种字面量可往返为正确的 `Value`；
- `0` 和空字符串为真；
- NUL、high byte、invalid UTF-8 不被丢失；
- 非法 number 和未闭合字符串返回结构化错误。

**暂不支持**：变量、表达式、注释、完整转义和多 Token 输入。

**专题映射**：`ch02-runtime/value.md`、`ch02-runtime/byte-string.md`。

### v0.2：Lexer 与 Token

**快照目录**：`book/code/v02-lexer/`

**上一版状态**：只能用临时读取器识别单个字面量。

**本版目标**：删除临时字面量读取器，用 byte-oriented Lexer 把完整 Lua 源码转换为
带位置信息的 Token 流。

**Lua 语义**：关键字、名字、数字、短字符串、长字符串、单/多字符运算符、单行/长
注释、空白、Lua 换行规则和输入结束。

**Rust 学习点**：`&'source [u8]`、生命周期、切片窗口、`Option<Token>` 预读、
状态机、源码位置和不可变输入。

**主要代码变化**：新增 `TokenType`、`Token`、`SourceLocation`、`Lexer`；
`Interpreter::run` 暂时输出 Token 流；删除 v0.1 的临时扫描代码。

**章节安排**：Token 的职责；借用源码而不复制；lookahead；字符串/数字状态机；错误位置。

**运行实验**：对包含 local、注释、长字符串和运算符的脚本输出 Token 表。

**测试与验收**：

- 关键字与同名前缀标识符能够区分；
- `\n`、`\r`、`\r\n`、`\n\r` 的行列行为被锁定；
- 转义、长括号层级、无效数字和未结束输入有明确错误；
- Token snapshot 与手写期望完全一致。

**暂不支持**：AST、优先级和语句执行。

**专题映射**：`ch03-compiler/lexer.md`。

### v0.3：Parser 与最小 AST

**快照目录**：`book/code/v03-parser/`

**上一版状态**：有平坦 Token 流，没有语法结构。

**本版目标**：把表达式 Token 组合成递归 AST，并用一个教学用 AST evaluator 执行。

**Lua 语义**：字面量、名字、括号、一元 `-`/`not`/`#`、二元运算符，以及 Lua 5.1
优先级和结合性。

**Rust 学习点**：递归 enum、`Box<Expr>`、节点位置、precedence climbing、
`Result<T, ParseError>` 和 `?`。

**主要代码变化**：新增 `Expr` 节点族、`Parser`、优先级表和 AST evaluator；
解释器输出 AST 与求值结果两种模式。

**章节安排**：为什么 Token 不够；递归类型为何需要 Box；优先级；结构化语法错误。

**运行实验**：比较 `1 + 2 * 3` 与 `(1 + 2) * 3` 的 AST 和结果。

**测试与验收**：

- 优先级、左/右结合和括号改变结构的测试通过；
- AST 每个节点保留正确行列；
- malformed expression、缺右括号和非法前缀不会 panic；
- evaluator 能处理 v0.1 已支持的值。

**暂不支持**：完整表达式语义、statement、bytecode 和变量绑定。

**专题映射**：`ch03-compiler/parser.md`。

### v0.4：完整表达式语义

**快照目录**：`book/code/v04-expressions/`

**上一版状态**：AST 已建立，但运算规则仍是最小子集。

**本版目标**：在进入 VM 前完成 Lua 表达式的可观察语义，建立一组以后 VM 必须保持
一致的 evaluator oracle。

**Lua 语义**：算术、比较、逻辑短路、`and/or` 返回原值、拼接、长度、数值转换和
类型错误；只有 nil/false 为假。

**Rust 学习点**：值分发、短路控制流、借用临时值、错误分类和运算 helper 复用。

**主要代码变化**：补齐二元/一元节点；新增 `RuntimeError`、number/string coercion、
Lua equality/order 和 short-circuit evaluator。

**章节安排**：Lua 真值；为什么 and/or 不返回 bool；运算快速路径；错误值与错误文本。

**运行实验**：执行一组算术、逻辑、比较与字符串拼接表达式。

**测试与验收**：

- 结果与官方 Lua 5.1 代表性输出一致；
- 短路右侧的故障表达式不会被执行；
- NaN、除零、跨类型比较和错误类别被明确记录；
- evaluator 测试成为 v0.5 VM 迁移的回归基线。

**暂不支持**：statement、变量、函数、Table 和 Metatable。

### v0.5：Bytecode 与最小寄存器 VM

**快照目录**：`book/code/v05-bytecode-vm/`

**上一版状态**：Parser 直接调用 AST evaluator。

**本版目标**：迁移为 `Lexer → Parser → AST → Codegen → Proto → Register VM`，
同时第一次建立与生产项目相似的 crate 边界。

**Lua/VM 能力**：常量表、固定 32 位指令、寄存器、`LOADK`、`LOADBOOL`、
`LOADNIL`、`MOVE`、表达式 opcode、条件跳转和 `RETURN`。

**Rust 学习点**：Cargo workspace、crate API、位掩码/位移、寄存器分配、dispatch loop、
状态与不可变 Proto 分离。

**架构变化**：

```text
lua_core      Value、ByteString、Proto
lua_compiler  Lexer、Parser、AST、Opcode、Codegen
lua_vm        Stack、LuaState、execute
lua_app       CLI 与流水线装配
```

**章节安排**：为什么从 AST evaluator 迁移；Lua 5.1 指令布局；寄存器；最小 VM；
workspace 边界。

**运行实验**：用 text 模式打印字节码，再执行并与 v0.4 evaluator 结果比较。

**测试与验收**：

- opcode 编码/解码和边界值往返；
- codegen 产生合法寄存器与常量索引；
- v0.4 全部表达式结果在 VM 中保持一致；
- 无效 opcode、越界常量和寄存器返回错误而不是 panic。

**暂不支持**：变量作用域、调用帧、完整 38 opcode。

**专题映射**：`ch03-compiler/bytecode.md`、`ch04-vm/register-vm.md`。

### v0.6：变量、赋值与作用域

**快照目录**：`book/code/v06-variables-scope/`

**上一版状态**：VM 只能执行没有名字绑定的表达式。

**本版目标**：加入 statement、局部/全局变量和词法作用域，使脚本第一次能保存并更新状态。

**Lua 语义**：global、local、普通/多重赋值、block local、遮蔽、`do ... end`、
未定义 global 返回 nil。

**Rust 学习点**：符号表、作用域栈、可变借用、寄存器所有权、RAII 风格作用域退出。

**主要代码变化**：新增 statement AST、local binder、scope、lvalue/rvalue、
`GETGLOBAL`、`SETGLOBAL`、局部寄存器生命周期和 global table。

**章节安排**：名字何时解析；local 为什么可以直接映射寄存器；并行赋值；作用域释放。

**运行实验**：变量遮蔽、值交换和 global/local 混合脚本。

**测试与验收**：

- `a,b=b,a` 使用赋值前值；
- 局部遮蔽退出后恢复外层绑定；
- 未定义 global 为 nil，未初始化 local 为 nil；
- 超出寄存器/常量限制产生结构化编译错误。

**暂不支持**：条件、循环、Table 和函数。

### v0.7：控制流

**快照目录**：`book/code/v07-control-flow/`

**上一版状态**：脚本只能顺序执行 statement。

**本版目标**：用跳转与回填实现结构化控制流。

**Lua 语义**：`if/elseif/else`、`while`、`repeat until`、numeric for、`break`、
`do` 和嵌套 block。

**Rust 学习点**：显式控制流状态、patch list、循环上下文、作用域退出与错误上下文。

**主要代码变化**：增加控制流 AST、`JMP`、`TEST`、`FORPREP`、`FORLOOP`、
跳转占位/回填和 break target stack。

**章节安排**：结构化语句如何降级为 pc；前向跳转；循环 back edge；break 与局部变量。

**运行实验**：FizzBuzz、嵌套循环和 repeat-until。

**测试与验收**：

- if 分支只执行一个路径；
- while/repeat 对条件检查位置正确；
- break 跳到最近循环并正确退出局部作用域；
- numeric for 正/负 step 和零 step 边界有明确行为。

**暂不支持**：generic for、Table 和函数调用。

### v0.8：Table

**快照目录**：`book/code/v08-table/`

**上一版状态**：值只有立即值和字符串，无法表达 Lua 对象图。

**本版目标**：实现 Lua 最核心的复合对象，并首次面对共享可变状态和环引用。

**Lua 语义**：数组/哈希混合表、构造器、`t[k]`、`t.name`、索引赋值、写 nil 删除、
nil/NaN 键错误和对象身份。

**Rust 学习点**：`Vec<Value>`、`HashMap<Value, Value>`、`Rc`、`RefCell`、
内部可变性、借用冲突和引用环。

**主要代码变化**：新增 `Table`、`TableRef`、Value::Table、table field AST、
`NEWTABLE`、`GETTABLE`、`SETTABLE`、`SETLIST`。

**章节安排**：Table 为什么既不是 Vec 也不是 HashMap；共享对象；RefCell 风险；
自引用为何会泄漏。

**运行实验**：数组、字典、混合构造器和 `t.self=t`。

**测试与验收**：

- 1-based array、稀疏整数和字符串键行为正确；
- 写 nil 删除键；nil/NaN 键报错；
- 同一个 Table 的两个 Value 观察到相同更新；
- 明确记录自引用无法由 Rc 回收，作为 v0.13 的动机。

**暂不支持**：Metatable、真实 GC 和迭代器。

**专题映射**：`ch02-runtime/table.md`。

### v0.9：函数与调用帧

**快照目录**：`book/code/v09-functions/`

**上一版状态**：只有顶层 Proto 和单一寄存器窗口。

**本版目标**：支持单返回值 Lua 函数与递归，建立完整调用帧模型。

**Lua 语义**：函数声明、local function、函数表达式、参数、单返回、递归、方法声明和
冒号调用语法糖。

**Rust 学习点**：对象身份、嵌套 Proto、调用栈、base/top、递归深度与错误回溯。

**主要代码变化**：Value::Function、Function/Proto 层级、CallInfo、`CALL`、`RETURN`、
函数 codegen 和 source/line metadata。

**章节安排**：函数值与代码原型；栈窗口；参数布局；递归；方法 self 的语法糖。

**运行实验**：阶乘、递归 Fibonacci 和对象方法调用。

**测试与验收**：

- 普通、嵌套和递归调用结果正确；
- 参数缺失补 nil，多余参数暂时忽略；
- 方法调用自动插入 self；
- 超过调用深度返回 Lua 错误并保留调用来源。

**暂不支持**：多返回、vararg、closure capture、tail call。

### v0.10：多返回、vararg 与迭代器

**快照目录**：`book/code/v10-multiret-vararg/`

**上一版状态**：函数只能返回一个值，调用结果宽度固定。

**本版目标**：实现 Lua 最容易出错的值列表调整规则，并完成 generic for 调用协议。

**Lua 语义**：多返回展开/折叠、最终表达式规则、vararg、Lua 5.1 `arg` 表、tail call、
generic for、`next`、`pairs`、`ipairs`。

**Rust 学习点**：动态结果宽度、栈区间、sentinel 值、边界条件和批量移动。

**主要代码变化**：扩展 `CALL`/`RETURN` B/C 字段、`VARARG`、`TAILCALL`、
`TFORLOOP`、结果列表调整和 iterator triple。

**章节安排**：为什么“最后一个表达式”特殊；MULTRET；vararg 帧；尾调用；generic for。

**运行实验**：交换、多返回透传、可变参数求和和自定义迭代器。

**测试与验收**：

- 赋值、参数、return、table constructor 的多返回矩阵通过；
- 非最终调用只产生一个结果；
- vararg 在嵌套调用中不串帧；
- generic for 正确处理 iterator/state/control 三元组。

**暂不支持**：闭包捕获和 coroutine yield。

### v0.11：Closure 与 Upvalue

**快照目录**：`book/code/v11-closures-upvalues/`

**上一版状态**：嵌套函数不能捕获外层 local。

**本版目标**：实现 open/closed Upvalue，使局部变量可以比创建它的调用帧活得更久。

**Lua 语义**：词法捕获、兄弟闭包共享、局部递归函数、循环变量捕获和关闭 Upvalue。

**Rust 学习点**：位置与值分离、共享状态、身份保持、生命周期迁移和间接引用。

**主要代码变化**：Upvalue descriptor、open-upvalue 链、Value::Function 捕获列表、
`CLOSURE`、`GETUPVAL`、`SETUPVAL`、`CLOSE`。

**章节安排**：为什么复制值不等于捕获变量；open/closed；共享；作用域退出与 close。

**运行实验**：计数器工厂、两个闭包共享一个变量、函数返回后继续访问。

**测试与验收**：

- sibling closures 观察相同写入；
- close 后变量值保持且不再访问旧栈槽；
- 地址/栈槽复用不会串到旧 Upvalue；
- 循环和 break 路径触发正确的 CLOSE。

**暂不支持**：真实 GC、跨 coroutine Upvalue owner。

### v0.12：Metatable 与元方法

**快照目录**：`book/code/v12-metatables/`

**上一版状态**：运算和索引只有内建类型快速路径。

**本版目标**：加入 Lua 的动态对象协议，在不破坏快速路径的前提下支持用户定义行为。

**Lua 语义**：`__index`、`__newindex`、`__call`、算术、比较、拼接、长度、`__tostring`，
以及 primitive metatable。

**Rust 学习点**：动态分发、快速/慢速路径、递归保护、错误分类和回调重入。

**主要代码变化**：metatable 存储、元方法名缓存、get/set table 慢路径、callable 分发和
统一 metamethod invocation。

**章节安排**：Metatable 不是继承；索引链；二元元方法选择；回调可能再次进入 VM。

**运行实验**：代理表、向量加法、可调用 Table 和自定义 tostring。

**测试与验收**：

- Table、Function 和 primitive 元方法覆盖代表场景；
- `__index/__newindex` 支持函数与 Table 两种形式；
- 循环链在固定上限处给出确定错误；
- 元方法错误能沿原调用边界传播。

**暂不支持**：`__gc`、弱表和 GC 驱动的生命周期。

### v0.13：Runtime 所有权与完整 GC

**快照目录**：`book/code/v13-runtime-gc/`

**上一版状态**：Table、Function、Upvalue 使用 Rc/RefCell，环引用不能回收。

**本版目标**：迁移到 Runtime 独占 Heap 的受管理对象图，实现 stop-the-world mark/sweep
和确定性 shutdown。

**Lua 语义**：GC 对用户透明；可达对象保留；不可达环可回收；registry、全局表、VM
栈和临时结果都是根。

**Rust 学习点**：对象图、arena/heap owner、typed handle、ObjectId、受控 unsafe、
stale/foreign handle 校验和 Drop 顺序。

**主要代码变化**：

- 用 `GcRef<T>` 替换 `Rc<RefCell<T>>`；
- 引入 Heap/Collector、对象 header、mark bit、root tracer 和 sweep；
- Runtime 成为 Heap、LuaState、StringPool 和 registry 的唯一 owner；
- 所有 managed-object 读取经过 checked scoped access。

**章节安排**：Rust 所有权为何不能代替动态 GC；根；mark；sweep；unsafe 不变量；shutdown。

**运行实验**：创建自引用 Table、移除最后一个根并触发 full collection。

**测试与验收**：

- 环状对象能被真实回收并反映在 allocation count；
- reachable、temporary root 和 open Upvalue 保持存活；
- foreign/stale/type-mismatched GcRef 被拒绝；
- shutdown 后 managed allocation 为零，final Drop 不访问已释放 Heap。

**暂不支持**：增量回收、write barrier、弱表、Finalizer 和 automatic GC。

### v0.14：增量 GC、弱表与 Finalizer

**快照目录**：`book/code/v14-incremental-gc/`

**上一版状态**：只有显式 full collection，长暂停不可控。

**本版目标**：把 GC 扩展为可分步推进的状态机，并完成 Lua 可观察的弱引用和 Finalizer。

**Lua 语义**：`collectgarbage`、weak key/value、userdata `__gc`、resurrection、再次死亡和
自动回收 checkpoint。

**Rust 学习点**：多阶段状态机、tri-color invariant、write barrier、allocator accounting、
回调重入、失败恢复和幂等清理。

**主要代码变化**：增量 phase/work unit、8-family mutation barrier、weak reconciliation、
finalizer queue、resurrection propagation、live/peak/total accounting。

**章节安排**：为什么 mark/sweep 不能随便暂停；写屏障；弱表；Finalizer 两阶段处理；计账。

**运行实验**：逐步回收对象图，观察 phase 和弱表条目变化。

**测试与验收**：

- full 与 incremental 对同一根图得到相同存活集合；
- 黑对象指向白对象时 barrier 保持不变量；
- weak k/v/kv、resurrection 和二次死亡测试通过；
- allocation threshold 能在 VM safe point 触发自动回收。

**暂不支持**：Coroutine 跨 state 根与 yield 中的回调调度。

### v0.15：Coroutine

**快照目录**：`book/code/v15-coroutines/`

**上一版状态**：Runtime 只有主 LuaState，调用不能暂停。

**本版目标**：把 LuaState 变成可由 Runtime 管理、暂停和恢复的执行实体。

**Lua 语义**：`create`、`resume`、`yield`、`status`、`running`、`wrap`，resume 参数、yield
结果、错误传播和 nested coroutine。

**Rust 学习点**：可暂停状态机、StateHandle、owner turn、mailbox/request-response、
跨状态调度和故障清理。

**主要代码变化**：StateArena、Thread object、activation stack、deferred call frame、
resume/yield 协议和 coroutine roots。

**章节安排**：为什么不能用普通递归调用实现 yield；保存 pc/stack/frame；owner；GC roots。

**运行实验**：生成器、双向传值、wrap 和 nested resume。

**测试与验收**：

- 基础状态转换与官方 Lua 代表行为一致；
- yield/resume 参数和多返回保持顺序；
- 错误只终止目标 coroutine 并返回正确 tuple；
- suspended closure、GC、1000 层 resume/wrap 和 shutdown 清理通过。

**暂不支持**：完整标准库 callback suspension 和 debug hook。

### v0.16：核心标准库

**快照目录**：`book/code/v16-core-stdlib/`

**上一版状态**：语言可执行，但常用能力仍需手写宿主入口。

**本版目标**：增加统一 Rust host-function ABI，并实现不依赖复杂平台资源的核心标准库。

**Lua 语义**：base、math、string、table 和 coroutine 库的 Lua 5.1 公开函数；参数检查、
返回 tuple、错误值和 callback suspension。

**Rust 学习点**：函数适配器、trait/回调、typed argument decoder、API 合同和注册表。

**架构变化**：增加 `lua_stdlib` crate；库函数通过窄 `VmContext` 使用 Runtime 服务，
不得直接持有 managed-object 裸引用。

**章节安排**：Rust 函数如何变成 Lua 函数；参数与错误；byte string API；回调与 yield。

**运行实验**：排序、字符串 pattern、数学函数、pcall/xpcall 和 coroutine 组合。

**测试与验收**：

- 每个注册函数至少有成功、边界、错误测试；
- string 函数保留 NUL/high byte；
- table.sort comparator、gsub replacement 等 callback 可 yield/报错；
- 代表性 API 与官方 Lua 进行进程级差分。

**暂不支持**：io、os、package、debug 和平台资源。

### v0.17：宿主标准库与 Lua 模块

**快照目录**：`book/code/v17-host-stdlib-modules/`

**上一版状态**：核心库已存在，但没有文件、系统、模块和调试能力。

**本版目标**：完成语言运行时范围内的宿主库和纯 Lua 模块加载链路。

**Lua 语义**：io、os、package、debug；Lua 文件模块；`package.path`、preload、require、
module cache、循环加载与错误传播。

**Rust 学习点**：文件句柄、Userdata resource、Drop/drain、路径与平台抽象、缓存状态、
宿主错误映射。

**主要代码变化**：File Userdata、host IO service、package searcher、loader protocol、
debug stack inspection 和 Runtime resource drain。

**章节安排**：资源为何不同于普通 GC 对象；文本/字节/路径边界；require 状态机；平台差异。

**运行实验**：读写文件、加载两个互相引用的 Lua 模块、打印 traceback。

**测试与验收**：

- 文件 close/flush/seek、错误 tuple 和重复 close 行为确定；
- require 缓存、preload、路径查找和循环模块测试通过；
- debug info/upvalue/local 代表 API 有回归；
- Runtime shutdown 先执行 Finalizer，再 drain 资源，最后释放 Heap。

**暂不支持**：`package.loadlib`、动态 native module、完整 locale/平台近似对齐。

### v0.18：CLI、REPL 与字节码工具

**快照目录**：`book/code/v18-cli-tools/`

**上一版状态**：解释器能力完整，但工具入口和外部可观察合同尚未系统化。

**本版目标**：把库代码变成可交付的 Lua 工具链，并建立进程级验证基础设施。

**Lua/工具能力**：脚本、stdin、`-e`、`-l`、`-i`、`--`、arg table、多行 REPL、
错误恢复、history/meta/completion、text/JSON bytecode、CFG 和 trace。

**Rust 学习点**：多 binary crate、终端状态、结构化输出、进程 runner、timeout、
stdout/stderr/exit/side-effect 断言。

**架构变化**：增加 `lua_bytecode`；`lua_app` 拆分 CLI/REPL 模块；增加 fixture manifest
和统一 runner。

**章节安排**：CLI 是语言合同的一部分；REPL 不完整输入；稳定 JSON schema；进程差分。

**运行实验**：脚本执行、stdin、交互式多行函数、bytecode JSON 和 VM trace。

**测试与验收**：

- CLI 参数矩阵、REPL 恢复和 bytecode schema 测试通过；
- runner 捕获 raw stdout/stderr/exit/timeout 和文件副作用；
- 工具错误不污染 stdout；
- Windows/Unix EOL 只在分析层规范化，raw 结果始终保留。

**暂不支持**：v1.0 全量 compatibility gate 与发布承诺。

### v1.0：Lua 5.1 语言运行时兼容收口

**快照目录**：`book/code/v100-lua51-runtime/`

**上一版状态**：语言、运行时、标准库和工具链均存在，但“有实现”不等于“兼容”。

**本版目标**：不再增加大类功能，集中补齐证据、修复差异并发布明确的 Lua 5.1 语言
运行时能力边界。

**兼容范围**：完整 Lua 5.1 源码语法、38 opcode、Value/Table/Function/Closure/Upvalue、
Metatable、GC、Coroutine、九类标准库、Lua 模块、CLI、REPL 和 bytecode/trace 工具。

**Rust 学习点**：双 oracle、差分测试、可观察合同、deviation governance、性能基线、
发布制品和长期维护。

**主要工作**：

- 建立官方 Lua 5.1.5 与固定 C++ oracle；
- 对 opcode 建立 unit + Lua behavior + bytecode/trace 三重证据；
- 执行 official applicable suite、fixture manifest 和语义差分；
- 所有差异必须修复或登记固定 NOTE，禁止静默近似；
- 生成能力矩阵、限制、性能基线、迁移说明和 release checklist；
- 将教学快照概念逐项映射到仓库生产实现。

**章节安排**：什么是兼容；如何选择 oracle；原始/规范化差异；失败聚类；发布门。

**运行实验**：同一 fixture 同时运行 Rust、官方 Lua 和固定 C++，输出结构化差分报告。

**测试与验收**：

- Debug/Release workspace tests、Clippy、rustdoc 和安全合同门通过；
- official applicable suite 达到项目声明门槛，排除项逐一说明；
- fixture stdout/stderr/exit/timeout/side effect 无未登记差异；
- bytecode 与 VM trace 代表集和逐 opcode 矩阵通过；
- GC/Coroutine/Finalizer/IO 的生命周期与故障测试通过；
- 发布文档不把 v1.1+ 能力描述为 v1.0 已支持。

**明确不包含**：binary chunk、C API/ABI 和动态 native module。

**专题映射**：`ch05-engineering/testing.md` 以及 `docs/rust_migration/` 中的 RFC、Phase
报告和 deviation log。

## 6. v1.0 后进阶路线

这些版本延续同一教学方法，但不阻塞 Lua 5.1 语言运行时 v1.0：

#### v1.1：Binary Chunk

- Lua 5.1 chunk 格式、端序/宽度、serializer、deserializer 和 verifier；
- `string.dump`、load binary mode、损坏/恶意 chunk 防护；
- 与官方 `luac` 和 Lua loader 的 binary parity。

#### v1.2：C API/ABI

- `lua_capi` crate、稳定 headers、State/stack/type/table/function/userdata API；
- lauxlib/lualib、allocator/panic callback 和 C consumer；
- ABI、symbol、sanitizer 和跨语言生命周期验证。

#### v1.3：动态原生模块

- `package.loadlib`、平台动态库抽象、symbol lookup 和 unload policy；
- native module 错误、安全边界和跨平台测试矩阵。

#### v1.4：生产治理与 SDK

- 沙箱、指令/内存/时间配额、owner-thread、metrics 和 worker；
- benchmark、coverage、跨平台制品、版本策略和发布治理。

## 7. 当前章节迁移映射

现有章节不删除，后续移动到“专题深入”区域，并由版本主线链接：

| 当前章节 | 主线入口 | 后续角色 |
|---|---|---|
| `ch02-runtime/value.md` | v0.1 | Value 深入与生产实现对照 |
| `ch02-runtime/byte-string.md` | v0.1 | 任意字节、NUL、UTF-8 边界 |
| `ch02-runtime/table.md` | v0.8、v0.13 | 混合存储、共享对象与 GC 迁移 |
| `ch03-compiler/lexer.md` | v0.2 | byte-oriented lexer 深入 |
| `ch03-compiler/parser.md` | v0.3 | AST、Box 与错误恢复深入 |
| `ch03-compiler/bytecode.md` | v0.5 | 38 opcode、编码和 codegen 深入 |
| `ch04-vm/register-vm.md` | v0.5、v0.9 | dispatch、栈和调用帧深入 |
| `ch05-engineering/testing.md` | v1.0 | fixture、双 oracle 与 CI |
| `ch05-engineering/next.md` | v0.11–v0.17 | 拆分为闭包、GC、协程和标准库版本 |

后续 `SUMMARY.md` 的目标层级：

```text
开始阅读
版本主线
  v0.0 ... v0.18
  v1.0
专题深入
  Value / ByteString / Table
  Lexer / Parser / Bytecode
  VM / Testing
工程记录
  RFC / Phase / Deviation
```

## 8. 实施顺序

版本代码与文章按以下批次交付，每个批次完成后都必须能够独立发布网站：

1. **基础闭环**：v0.0–v0.4，完成输入、Value、Lexer、Parser 和表达式 evaluator；
2. **编译执行**：v0.5–v0.7，完成 workspace、Bytecode、VM、变量和控制流；
3. **Lua 对象模型**：v0.8–v0.12，完成 Table、函数、多返回、Closure 和 Metatable；
4. **运行时生命周期**：v0.13–v0.15，完成 full/incremental GC 与 Coroutine；
5. **生态与工具**：v0.16–v0.18，完成标准库、模块、CLI、REPL 和工具；
6. **兼容收口**：v1.0，完成双 oracle 证据与发布文档；
7. **进阶能力**：v1.1–v1.4，不影响 v1.0 的语言运行时教学闭环。

每个批次开始前先冻结该批次的示例脚本和验收表；实现过程中可以补充测试，但不能
通过降低既定验收口径来宣布版本完成。

## 9. 完成定义

一个教学版本只有同时满足以下条件才能在目录中标记为“完成”：

- 代码快照能够在干净环境独立构建；
- 章节中的全部命令、输出和源码 include 已验证；
- 自动测试覆盖本版新增语义与失败边界；
- “暂不支持”没有与真实代码矛盾；
- 从上一版的架构变化被明确讲解；
- 读者不阅读未来版本也能完成本版练习；
- 对官方 Lua 的兼容描述与证据强度一致；
- 本版本的专题深入链接和下一版本动机完整。

整本版本主线只有在 v1.0 验收门通过、v1.1+ 范围被清晰隔离后，才可以对外描述为
“从零实现 Lua 5.1 语言运行时”的完整课程。
