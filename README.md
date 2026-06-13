# lua_rust — Lua 5.1 Interpreter in Rust

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

一个用纯 Rust 编写的 **Lua 5.1 解释器**，完整实现寄存器虚拟机、编译器前端、标记-清除 GC 和标准库。

---

## 目录

- [特性](#特性)
- [快速开始](#快速开始)
- [项目结构](#项目结构)
- [Crate 说明](#crate-说明)
- [架构概览](#架构概览)
- [开发](#开发)
- [文档](#文档)
- [许可证](#许可证)

---

## 特性

- **完整的 Lua 5.1 语义**: nil/boolean/number/string/table/function/userdata/thread 全部值类型
- **寄存器虚拟机**: 32 位指令格式，38 条 opcode，RK 操作数编码
- **标记-清除 GC**: 三色标记算法，弱表支持，终结器与复活机制，写屏障
- **字符串驻留**: 相同内容的字符串在内存中仅存一份，比较简化为指针比较
- **编译器前端**: 完整的 Lexer → Parser → AST → CodeGenerator 管线
- **标准库**: base、math、string、table、io、os、coroutine、debug、package
- **交互式 REPL**: 支持行编辑、历史记录
- **字节码工具**: 编译 Lua 源码并输出字节码（text / JSON 格式）

---

## 快速开始

### 环境要求

- Rust **1.96+** (stable toolchain)
- Windows x64-msvc（主要开发平台）

### 安装与构建

```powershell
# 克隆仓库
git clone <repo-url>
cd lua_rust

# 构建全部 workspace
cargo build --workspace

# 运行全部测试
cargo test --workspace

# 构建文档
cargo doc --no-deps --open
```

### 运行 Lua 脚本

```powershell
# 执行 Lua 脚本文件
cargo run -p lua_app -- path/to/script.lua

# 启动交互式 REPL
cargo run -p lua_app

# 查看字节码
cargo run -p lua_bytecode -- path/to/script.lua --format=json
```

---

## 项目结构

```
lua_rust/
├── Cargo.toml                  # Workspace 根配置
├── rust-toolchain.toml         # Rust 工具链固定版本
├── README.md
├── .cargo/
│   └── config.toml             # 构建配置
├── .github/
│   └── workflows/
│       └── ci.yml              # CI 流水线
├── crates/
│   ├── lua_core/               # 运行时核心库
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── types.rs        # 基础类型、ValueType、GcObjectType
│   │   │   ├── value.rs        # Value 枚举 — Lua 值的统一表示
│   │   │   ├── gc/             # 垃圾回收模块
│   │   │   │   ├── mod.rs
│   │   │   │   ├── header.rs   # GcObjectHeader（侵入式链表节点）
│   │   │   │   ├── gc_ref.rs   # GcRef<T>（GC 引用安全句柄）
│   │   │   │   ├── gc_object.rs # GcObject trait
│   │   │   │   ├── collector.rs # GarbageCollector
│   │   │   │   └── strategy.rs  # GCStrategy trait + 内置策略
│   │   │   ├── gc_string.rs    # GcString（驻留字符串）
│   │   │   ├── string_pool.rs  # StringPool（字符串驻留池）
│   │   │   ├── table.rs        # Table（数组 + 哈希）
│   │   │   ├── function.rs     # Function + Proto（函数与字节码容器）
│   │   │   ├── upvalue.rs      # Upvalue（闭包捕获变量）
│   │   │   ├── thread.rs       # Thread（协程）
│   │   │   └── userdata.rs     # Userdata（用户自定义数据）
│   │   └── tests/
│   ├── lua_compiler/           # 编译器前端
│   │   └── src/
│   │       ├── opcode.rs       # OpCode 枚举 + Instruction 编解码
│   │       ├── lexer/          # 词法分析器
│   │       ├── parser/         # 语法分析器
│   │       ├── ast/            # 抽象语法树
│   │       └── codegen/        # 字节码生成器
│   ├── lua_vm/                 # 虚拟机核心
│   │   └── src/
│   │       ├── state/          # LuaState、GlobalState、Stack、CallInfo
│   │       ├── execute.rs      # 主 dispatch 循环（38 条指令）
│   │       ├── ops.rs          # 算术/比较辅助
│   │       ├── call.rs         # 调用/返回辅助
│   │       └── trace.rs        # 执行 trace / 调试
│   ├── lua_stdlib/             # 标准库
│   │   └── src/
│   │       ├── catalog.rs      # 库注册表
│   │       ├── base.rs         # base 库
│   │       ├── math.rs         # math 库
│   │       ├── string.rs       # string 库
│   │       ├── table.rs        # table 库
│   │       ├── io.rs           # io 库
│   │       ├── os.rs           # os 库
│   │       ├── coroutine.rs    # coroutine 库
│   │       ├── debug.rs        # debug 库
│   │       └── package.rs      # package 库
│   ├── lua_app/                # CLI 应用入口
│   │   └── src/main.rs         # REPL + 脚本执行器
│   └── lua_bytecode/           # 字节码工具
│       └── src/main.rs         # Lua 源码 → 字节码输出
├── tests/
│   └── fixtures/               # Lua 测试脚本
├── tools/                      # 辅助脚本
│   ├── rust_env_check.ps1      # 开发环境检查
│   └── rust_quality_gate.ps1   # 质量门禁
└── docs/                       # 项目文档
    ├── glossary.md             # 术语表
    └── rust_migration/         # 类型映射与偏差记录
```

---

## Crate 说明

| Crate | 类型 | 描述 |
|---|---|---|
| `lua_core` | lib | 运行时核心：类型系统、Value、GC 基础设施、字符串驻留、Table、Function、Upvalue、Thread、Userdata |
| `lua_compiler` | lib | 编译器前端：OpCode 指令集、词法分析、语法分析、AST 构建、字节码生成 |
| `lua_vm` | lib | 虚拟机：LuaState、值栈、CallInfo 调用帧、38 条指令 dispatch、调用返回、trace/debug |
| `lua_stdlib` | lib | 标准库：base、math、string、table、io、os、coroutine、debug、package 全部模块 |
| `lua_app` | bin | CLI 应用：交互式 REPL 和 Lua 脚本文件执行器 |
| `lua_bytecode` | bin | 字节码工具：编译 `.lua` 文件并以 text 或 JSON 格式输出字节码 |

### 依赖关系

```
lua_core          ← 基础层（无内部依赖）
  ↑
lua_compiler      ← 依赖 lua_core（使用类型系统和字符串池）
  ↑
lua_vm            ← 依赖 lua_core + lua_compiler
  ↑
lua_stdlib        ← 依赖 lua_core + lua_vm
  ↑
lua_app           ← 依赖全部 4 个库 crate
lua_bytecode      ← 依赖 lua_core + lua_compiler
```

---

## 架构概览

### 值系统

`Value` 是一个 Rust enum，统一表示所有 Lua 运行时值：

```rust
pub enum Value {
    Nil,                              // nil
    Boolean(bool),                    // true / false
    LightUserdata(GcRef<c_void>),     // 轻量用户数据
    Number(f64),                      // 浮点数
    String(GcRef<GcString>),          // 驻留字符串
    Table(GcRef<Table>),              // 表
    Function(GcRef<Function>),        // 函数
    Userdata(GcRef<Userdata>),        // 用户数据
    Thread(GcRef<Thread>),            // 协程
}
```

### 垃圾回收

采用三色标记-清除算法：

- **白色**: 未访问，可能被回收
- **灰色**: 已访问但未扫描子引用
- **黑色**: 已访问且已扫描所有子引用

GC 通过 `GarbageCollector` 管理侵入式对象链表，`GcRef<T>` 提供对 GC 对象的安全引用句柄。支持弱表（弱键/弱值）、终结器（`__gc` 元方法）和写屏障。

### 字符串驻留

`StringPool` 确保相同内容的字符串在内存中仅存一份。字符串创建时预计算 Lua 5.1 兼容的哈希值，后续比较直接通过指针相等完成。

### 编译器管线

```
Lua 源码
  → Lexer (词法分析) → Token 流
  → Parser (语法分析) → AST
  → CodeGenerator (代码生成) → Proto（字节码）
```

### 虚拟机

寄存器架构的字节码解释器，32 位定长指令，38 条 opcode：

```
MOVE  LOADK  LOADBOOL  LOADNIL
GETUPVAL  GETGLOBAL  GETTABLE
SETGLOBAL  SETUPVAL  SETTABLE
NEWTABLE  SELF
ADD  SUB  MUL  DIV  MOD  POW
UNM  NOT  LEN
CONCAT
JMP  EQ  LT  LE  TEST  TESTSET
CALL  TAILCALL  RETURN
FORLOOP  FORPREP  TFORLOOP
SETLIST  CLOSE  CLOSURE  VARARG
```

---

## 开发

### 命名规范

| 元素 | 规范 | 示例 |
|---|---|---|
| 类型 | `CamelCase` | `Value`, `StringPool`, `GcRef` |
| 方法与函数 | `snake_case` | `is_nil()`, `as_number()` |
| 常量 | `UPPER_CASE` | `NUM_OPCODES`, `WHITE0BIT` |
| 模块路径 | `snake_case` | `lua_core::gc::header` |

### Unsafe 使用原则

`unsafe` 严格限制在以下边界：

- GC 内部（侵入式链表操作）
- `GcRef<T>` 的裸指针解引用
- FFI / C API 边界

每个 `unsafe` 块必须附带 `// SAFETY:` 注释，说明依赖的不变量及其成立理由。

### 常用命令

```powershell
# 构建
cargo build --workspace
cargo build -p lua_core

# 测试
cargo test --workspace
cargo test -p lua_core

# 代码质量
cargo fmt --check
cargo clippy --workspace -- -D warnings

# 文档
cargo doc --no-deps --open

# 质量门禁
powershell -NoProfile -ExecutionPolicy Bypass -File tools/rust_quality_gate.ps1
```

---

## 文档

| 文档 | 说明 |
|---|---|
| [术语表](docs/glossary.md) | Lua 概念在本项目各模块中的位置 |
| [类型映射](docs/rust_migration/type_mapping_table.md) | 类型别名、枚举与 trait 速查 |

---

## 许可证

MIT License — 详见 [LICENSE](LICENSE) 文件。
