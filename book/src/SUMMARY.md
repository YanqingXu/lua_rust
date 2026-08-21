# Summary

[开始阅读](PREFACE.md)

# 版本主线

- [从 0 到 Lua 5.1](versions/README.md)
  - [v0.0 · 最小解释器骨架](versions/v00-skeleton.md)
  - [v0.1 · Lua Value 与字面量](versions/v01-value.md)
  - [v0.2 · Lexer 与 Token](versions/v02-lexer.md)
  - [v0.3 · Parser 与最小 AST](versions/v03-parser.md)
  - [v0.4 · 完整表达式语义](versions/v04-expressions.md)
  - [v0.5 · Bytecode 与最小寄存器 VM](versions/v05-bytecode-vm.md)
  - [v0.6 · 变量、赋值与作用域](versions/v06-variables-scope.md)
  - [v0.7 · 控制流](versions/v07-control-flow.md)
  - [v0.8 · Table](versions/v08-table.md)
  - [v0.9 · 函数与调用帧](versions/v09-functions.md)
  - [v0.10 · 多返回、vararg 与迭代器](versions/v10-multi-return-vararg.md)
  - [v0.11 · Closure 与 Upvalue](versions/v11-closure-upvalue.md)
  - [v0.12 · Metatable 与元方法](versions/v12-metatable.md)
  - [v0.13 · Runtime 所有权与完整 GC](versions/v13-runtime-gc.md)
  - [v0.14 · 增量 GC、弱表与 Finalizer](versions/v14-incremental-gc.md)
  - [v0.15 · Coroutine](versions/v15-coroutine.md)
  - [v0.16 · 核心标准库](versions/v16-core-stdlib.md)
  - [v0.17 · 宿主标准库与 Lua 模块](versions/v17-host-stdlib-modules.md)
  - [v0.18 · CLI、REPL 与字节码工具](versions/v18-cli-tools.md)
  - [v1.0 · Lua 5.1 语言运行时兼容收口](versions/v100-lua51-runtime.md)

# 专题深入

- [学习路线](ch01-start/roadmap.md)
- [搭建与运行成熟实现](ch01-start/setup.md)

## 运行时核心

- [01 · 用 enum 表示 Lua Value](ch02-runtime/value.md)
- [02 · Lua 字符串不等于 Rust String](ch02-runtime/byte-string.md)
- [03 · Table、所有权与共享对象](ch02-runtime/table.md)

## 编译器

- [04 · Lexer：从字节到 Token](ch03-compiler/lexer.md)
- [05 · Parser：用 AST 表达语法](ch03-compiler/parser.md)
- [06 · Bytecode：设计寄存器指令](ch03-compiler/bytecode.md)

## 虚拟机

- [07 · 寄存器虚拟机](ch04-vm/register-vm.md)

# 工程记录

- [08 · 用测试守住兼容性](ch05-engineering/testing.md)
- [下一站：闭包、GC 与协程](ch05-engineering/next.md)
