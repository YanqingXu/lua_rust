# Summary

[开始阅读](PREFACE.md)

# 第一部分：准备出发

- [学习路线](ch01-start/roadmap.md)
- [搭建与运行](ch01-start/setup.md)

# 第二部分：运行时核心

- [01 · 用 enum 表示 Lua Value](ch02-runtime/value.md)
- [02 · Lua 字符串不等于 Rust String](ch02-runtime/byte-string.md)
- [03 · Table、所有权与共享对象](ch02-runtime/table.md)

# 第三部分：把源码变成字节码

- [04 · Lexer：从字节到 Token](ch03-compiler/lexer.md)
- [05 · Parser：用 AST 表达语法](ch03-compiler/parser.md)
- [06 · Bytecode：设计寄存器指令](ch03-compiler/bytecode.md)

# 第四部分：执行 Lua

- [07 · 寄存器虚拟机](ch04-vm/register-vm.md)

# 第五部分：工程化

- [08 · 用测试守住兼容性](ch05-engineering/testing.md)
- [下一站：闭包、GC 与协程](ch05-engineering/next.md)
