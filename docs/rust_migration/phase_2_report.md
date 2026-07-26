---
status: partial
phase: 2
phase_name: Compiler
last_updated: 2026-07-26
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 2 Report: Compiler

## 结论

**状态：partial。** Lexer、Parser、AST、opcode metadata 和 CodeGen 管线已经能为
较广的项目内语料生成 `Proto`；这证明编译器是可运行原型，不证明 38 个 opcode、
错误分类、调试信息和极端输入已经与双 oracle 对齐。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | 无。尚没有编译器能力同时满足项目测试、官方 Lua 与固定 C++ oracle。 |
| `partial` | Lexer/token、Parser/AST、38-opcode metadata、CodeGen 与 Proto 生成。 |
| `not-started` | byte cursor 全链路、逐 opcode 机器可读 parity matrix，以及完整 malformed/extreme corpus artifact。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Lexer/token | `crates/lua_compiler/src/lexer.rs`、`token.rs` | lexer 模块内单元测试 | partial：入口按 Rust text/char 工作，不是 byte cursor |
| Parser/AST | `parser/`、`ast/` | `crates/lua_compiler/tests/parser_tests.rs` | partial：主语法路径有项目内覆盖，错误恢复和极限未完成 |
| Opcode encoding/metadata | `opcode.rs` 定义 Lua 5.1 的 38 个枚举项及字段 helper | opcode 单元测试 | partial：枚举存在不等于每项 binary parity 已证明 |
| CodeGen | `codegen/` 的 scope、register、jump、expression/statement emit | compiler tests 与可运行 Lua fixtures | partial：没有逐 opcode bytecode differential 完成证据 |
| Proto output | constants、nested proto、line/local/upvalue 字段由 `Proto` 承载 | compiler/VM tests | partial：完整性尚未逐字段与 oracle 比较 |

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| 源码和字符串 literal 不是 bytes | lexer 使用 `chars()`，source API 接收 `&str` | [NOTE-007](deviation_log.md#note-007-lua-字符串尚未采用任意字节表示)，M1.3 |
| 某些合法性/容量错误仍可 panic | `expr_emit.rs` 的 malformed method call、`jump.rs` 的 control structure too long | M2.1 |
| 38 opcode 没有逐项 unit + Lua behavior + bytecode differential 三重证据 | 当前测试按功能组织，缺少机器可读 parity matrix | M0.6、M2.2 |
| Proto 字段没有完整 oracle 报告 | 无 constants/nested/debug/source 的结构化三方 diff 结果 | M2.3 |
| malformed/extreme corpus 不完整 | 寄存器耗尽、嵌套/递归、长跳转和超大常量缺少验收 artifact | M2.1 |

## Oracle 与验收

- 语法和 chunk 语义比较官方 Lua 5.1.5；项目扩展和精确 bytecode/Proto 结构比较
  `lua_cpp@87c15e6`。
- Parser/CodeGen 单元测试通过只能支持“实现存在”；只有结构化 bytecode diff、
  VM trace 和 error-category 用例通过，才能支持“兼容”。
- 完成条件对应 M1.3、M2.1–M2.3 和 M2 总门槛；目前保持 `partial`。
