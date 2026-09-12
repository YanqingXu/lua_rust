# 无循环 break 的挂死与加载器诊断整改

在 REPL 错误阶段回归中发现，裸 `break` 能让 Rust 永久停在同一条指令。
固定 C++ `87c15e69ceb94eb74e28226ccbefb7e196635711` 会在代码生成阶段报告
`no loop to break`。这是独立的 P1 编译控制流缺陷，旧诊断字节快照及 REPL
修复后的快照都已出现 30 秒超时，相关进程均由探针回收。

## 根因与修复

两端 Parser 都正常生成 Break AST。Rust `emit_break` 在查不到当前函数的循环时，
仍生成等待回填的 `JMP -1`，而 `append_break_jump` 没有循环可登记就直接返回。
VM 的 `pc = pc + sBx + 1` 因此不断回到原 PC，末尾 RETURN 永远不可达。

代码生成现在先检查当前函数的可退出循环。缺失时记录 `no loop to break` 并立即
返回，不创建跳转。采用既有 `pending_error` 机制，避免外层 if/do/loop emitter
忽略局部 Result 后交付错误 Proto。合法 break 的 upvalue 关闭与回填路径保持不变。
子函数有独立的循环栈，不能退出父函数的循环；已记录的 break 错误不会被后续函数清除。

额外编译探针确认，较早子函数中的无循环错误会被后续直接返回的 lvalue 错误覆盖。
`generate_with_source_bytes` 现在保存 emit 的 Result，先返回已记录的首个错误，
再传播后续直接错误。在已记录 break 错误、随后发生直接错误的三类回归中，
结果与固定 C++ 一致；这不等价于整个编译器已经保证所有首错误的传播。

protected loadstring 回归进一步暴露错误来源：Rust 给无位置 codegen 错误加了
`[string "break"]:0:0:`，固定 C++ 则用 `loadstring:`。共享编译入口现在显式接收
`ChunkLoaderContext`，为无位置代码生成错误分别选择 `loadstring:`、`load:` 或
`loadfile: filename:`。五条调用路径均传入实际入口身份，dofile 使用 loadfile 的
错误合同。文件名以字节传入；不从 chunk name 标记或错误正文猜测入口。

已有 Parser 语法诊断、非零位置错误、reader 回调错误对象保持原合同；package
独立加载器的 oracle 分歧另有记录。本轮没有扩大到其他诊断正文或改动 VM 跳转语义。

## 定向证据

产物在 `target/main-repl-evidence/`：

- `continuation-tests/stage-and-whitespace-supplement.json` 保留两个修复前 Rust
  快照的裸 break 超时，以及固定 C++ 报错后继续处理输入的结果。
- 新增 compiler `break_scope_tests`，初轮 1/6 通过、5 个拒绝测试得到错误的成功
  Proto；修复后 6/6，通过首错误优先级补充后共 7/7。测试不执行已知的自跳字节码。
- 覆盖顶层、if/do、未调用函数、四类循环内的子函数、有效/无效函数顺序、同一
  GC/StringPool 上新 generator 恢复，以及合法循环内 break 的跳转回填。
- REPL、批量 `-e break`、protected loadstring 的真实子进程回归由
  `repl_continuation` 执行。首轮 18/19 通过，唯一失败为上述 loadstring 前缀，
  原失败日志保留；修复不通过放宽测试预期完成。

修复后的 REPL target 最终 19/19；两种错误经四种加载入口的原始 stdout、stderr、
退出码与固定 C++ 完全相同，含错误后恢复，证据在
`target/main-repl-evidence/loader-prefix-final/observations.json`。
最终联合完整 M0/M1 基础门、Debug/Release 全 targets 均通过，源码身份和完整
产物见 [当前整改状态](2026-09-12-remediation.md)。合法 while/repeat/numeric/generic
循环的 break、闭包捕获和随后 GC，在最终 Debug/Release/固定 C++ 实际执行通过，
证据 `target/repl-break-bytecode-evidence/valid-loop-execution/observations.json`。
裸 break 在官方 Lua REPL 的解析续行阶段与固定 C++ 有不同错误时机，本项目遵循
固定 C++ 的 codegen 阶段；不会将 stock 的表现冒称为相同。

## 后续发现的 P1：嵌套 emitter 吞掉直接错误

以下保留发现时证据。后续实现和验证已在
[嵌套 codegen 整改](2026-09-12-nested-codegen-remediation.md) 单独记录，不把两批源码混为一体。

冻结后的独立审核用 `loadstring` 编译、没有执行返回的 Proto，确认仍有另一项
缺陷：`do 1=2 end` 和 `if true then 1=2 end` 在 Rust 返回 function/nil，固定
C++ 返回 nil/`loadstring: Expression is not a valid lvalue`。
`do 1=2 end; local function f() break end` 则在 Rust 报较晚的 break 错误，
C++ 报较早的 lvalue 错误。原因是现有嵌套 emitter 的 `let _ = emit_block/statement`
丢弃直接 Result，本轮新增的 break pending_error 能避免 break 被吞，不能替代
所有直接错误的传播。

六个短进程的源码、快照 SHA 和原始输出保留在
`target/compiler-error-propagation-evidence/observations.json`。该发现时是下一优先修复项，
当时完整门通过也不能关闭它；必须传播嵌套 codegen 错误、保留最早诊断，并验证合法
嵌套循环/函数仍正确执行。该联合冻结源码尚未修改此路径，后续证据见上方链接。
