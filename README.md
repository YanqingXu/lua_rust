# lua_rust - Lua 5.1 Rust Migration

[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`lua_rust` 是 `lua_cpp` 的 Lua 5.1 解释器 Rust 迁移工程。Phase 0~5 框架已全部搭建，核心管线（编译→VM 执行→标准库）已打通，能运行基本 Lua 脚本。

最近审计日期：2026-06-20。

---

## 当前进度

| 范围 | 状态 | 说明 |
|---|---|---|
| Phase 0: Project Infrastructure | ✅ 已完成 | Rust workspace、6 个 crate 脚手架、CI workflow、质量脚本。 |
| Phase 1: Runtime Core | ✅ 已完成 | 类型系统、GC 基础设施、标记-清除 GC、全部核心对象模型。 |
| Phase 2: Compiler | ✅ 已完成 | 38 指令、Lexer、AST (27 节点)、Parser、CodeGen 完整管线。 |
| Phase 3: VM | ✅ 核心完成 | 38 opcode dispatch、数据移动/算术/控制流/循环/表/调用/闭包全部实现。16 个集成测试。 |
| Phase 4: Standard Library | 🏗️ 进行中 | Catalog 注册、base (print/type/tostring 已实现)、math/string/table 框架已就位。 |
| Phase 5: CLI / tools | ✅ 基本完成 | `lua_app` 脚本执行器 + 交互式 REPL、`lua_bytecode` 字节码 dumper。 |

---

## 已验证的功能

- `print("hello world")` ✅ — 端到端：Lexer→Parser→CodeGen→VM→C function
- `return 42` ✅ — 简单算术返回值
- `local a = 10; b = 20; return a + b` ✅ — 局部变量 + 算术
- `return "hello"` ✅ — 字符串常量
- `return 1 == 1` ✅ — 比较运算符
- `if true then x = 1 end` ✅ — if 语句
- `return not false` ✅ — 布尔运算
- `return "a" .. "b"` ✅ — 字符串拼接

---

## 测试统计

| Crate | 单元测试 | 集成测试 | 总计 |
|---|---|---|---|
| `lua_core` | 256 | 41 | 297 |
| `lua_compiler` | 111 | 66 | 177 |
| `lua_vm` | 6 | 16 | 22 |
| `lua_stdlib` | 0 | 0 | 0 |
| `lua_app` | 0 | 0 | 0 |
| `lua_bytecode` | 0 | 0 | 0 |
| **总计** | **373** | **123** | **496** |

所有测试在本地和 CI 中全部通过 (`cargo test --workspace`)。
C++ 基准：668 tests / 3404 assertions / 0 failures。

---

## CI 与 C++ 基准对齐

`.github/workflows/ci.yml` 当前包含：

| Job | 状态 | 说明 |
|---|---|---|
| `quality-gate` | 已配置 | Windows 上运行 fmt、clippy、build、nextest、doc。 |
| `cross-validate` | 待启用 | `if: false`，C++ 基准产物已构建，待启用。 |

---

## 已知限制 (Known Limitations)

| 领域 | 限制项 |
|---|---|
| **字符串驻留** | 未激活 StringPool interning；全局表查找使用内容回退（O(n) 遍历哈希表）。 |
| **C 函数 GC 访问** | `type()`/`tostring()` 当前打印到 stdout 而非返回栈上值（C 函数无法创建 GC 对象）。 |
| **CodeGen** | 表构造器 SETLIST/SETTABLE 发射未完整实现；跨函数 upvalue 解析未完成。 |
| **标准库** | 仅 base 库的 print/type/tostring 已实现。math/string/table 函数均为占位。io/os/coroutine/debug/package 未实现。 |
| **CLI** | REPL 不支持多行输入；lua_bytecode 不输出字符串常量内容。 |

---

## 快速开始

```powershell
cd lua_rust

cargo fmt --check
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace

# 运行 Lua 脚本
cargo run -p lua_app -- examples/hello.lua

# 字节码 dump
cargo run -p lua_bytecode -- examples/hello.lua
```

---

## Crate 说明

| Crate | 类型 | 职责 |
|---|---|---|
| `lua_core` | lib | 类型系统、GC、核心对象模型 (Table/Metatable/Proto/Function/Upvalue/Userdata/Thread)。297 个测试。 |
| `lua_compiler` | lib | Opcode、Lexer/Token、AST、Parser、CodeGen。177 个测试。 |
| `lua_vm` | lib | LuaState、Stack、CallInfo、38 opcode dispatch。22 个测试。 |
| `lua_stdlib` | lib | Catalog + base/math/string/table（base 已实现 print/type/tostring）。 |
| `lua_app` | bin | Lua 脚本执行器 + 交互式 REPL。 |
| `lua_bytecode` | bin | 字节码 text/JSON dump。 |

---

## 文档

| 文档 | 说明 |
|---|---|
| [Phase 1 进度报告](docs/rust_migration/phase_1_progress.md) | Phase 1 Runtime Core 整体进度跟踪。 |
| [偏差日志](docs/rust_migration/deviation_log.md) | Rust 与 C++ 基准之间已批准行为偏差的登记表。 |
| [类型映射表](docs/rust_migration/type_mapping_table.md) | C++ → Rust 类型映射速查表。 |
| [术语表](docs/glossary.md) | Lua 概念与项目术语说明。 |

---

## 许可证

MIT License - 详见 [LICENSE](LICENSE)。
