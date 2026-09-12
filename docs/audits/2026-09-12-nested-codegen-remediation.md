# 2026-09-12 嵌套编译错误传播整改

状态：本轮实现与验收完成，与 Windows popen 整改一同纳入本次提交。基于已推送的
`33aba8e`，本报告对应其后的修改；远程 CI 结果须按新提交独立核对。固定 C++ oracle 仍为
`87c15e69ceb94eb74e28226ccbefb7e196635711`，第二 oracle 为官方 Lua 5.1.5。

## 问题与实测基线

`do 1=2 end`、`if false then 1=2 end` 等源码中的无效赋值已被 codegen 发现，
但 if/elseif/else、while、repeat、numeric for、generic for、do 的七处调用
忽略了 `emit_block` 或 `emit_statement` 的 `Result`。加载器因此可能返回 function，
或者继续编译后被较晚的 `no loop to break` 错误覆盖。

修复前的 18 场景矩阵每个引擎只启动一个仅编译进程：Rust Debug/Release 的前
15 场景错误地返回 function，两个 direct-before-break 场景错误地报告较晚的 break；
另一个 break-before-direct 场景已正确。固定 C++ 全部拒绝并保持首个 codegen 错误。
stock 同样全部拒绝，但在 parser 阶段报告，文案和错误来源单独保存。
所有红矩阵进程正常退出，没有执行意外返回的 function，副作用计数均为 0。

旧二进制还运行了新增 fixture 和 reader 合同的安全前半段，断言在发现返回 function
后立即失败。旧版没有运行会实际执行非法 chunk 的 batch、script 或 REPL 反例。
原始数据位于 `target/compiler-error-propagation-evidence/current-work/` 的
`red-matrix.json` 与 `contracts-red-and-oracles.json`，含二进制快照、完整输出和退出码。

## 实现

- 六类控制结构 emitter 改为返回 `Result`，向上传递所有七处被忽略的错误。
  合法路径上的 opcode、跳转、作用域关闭顺序保持不变。
- 语句、代码块和控制条件/迭代器的边界检查已有的 `pending_error`，阻止失败表达式
  之后继续进入其他代码块。公共 `generate` 与 `compile_function` 保留首个错误。
- 子函数编译失败时先恢复外层 builder、寄存器和控制栈等上下文，再返回错误；
  失败的子 Proto 不得附加到父 Proto。入口检查也阻止后续兄弟函数附加 Proto。
- repeat 临时使用的局部 nil 初始化设置在错误路径同样恢复。

这不是对整个编译器状态做事务回滚：失败前上值解析设置的 captured 标记可能保留，
随后整个生成请求会失败。低层 `emit_statement` 的 String 错误也不单独承诺首错；
上述两个公共编译出口通过保留的 owned error 提供最终首错合同。

## 定向验证

- 编译器新增测试先得到 14 项中的 12 失败；修复后 14 项通过。继续增加同一语句内
  双向首错和兄弟 Proto 不附加回归，最终新增 17 项全部通过。
  连同既有 break 7 项、bytecode parity 33 项，共 57/57，通过后冻结生产代码。
- CLI 新增 5 项在 Debug/Release 都通过，覆盖 loadstring、reader、loadfile、dofile、
  batch 后续参数停止、script 全 chunk 拒绝、REPL 继续接收下一条命令。
  fixture 在每次拒绝后编译并执行合法闭包，再 GC 检查捕获值。
- 新增 `compiler_error_boundary` 两项普通测试通过：先成功分配子函数再失败的
  publication 临时根全部释放，partial graph 可全部回收；真实 Runtime 内 reader
  回调触发 GC 后诊断仍有效，随后合法闭包执行成功，关闭后对象、allocator live bytes、
  coroutine state 和资源清理错误均为 0。CI 增加该 target 的独立默认 Miri 步骤。
- 独立复核的六组仅编译反例与固定 C++ 原始输出/退出码一致；另有十二个同语句
  场景的固定 C++ 证据。复核报告分别保存在 `independent-p1-review/` 与
  `compiler-work/implementation-review.md`。

默认 Miri 的两项 boundary 测试通过，耗时 382.76 秒，未设置 `MIRIFLAGS`，没有
关闭别名检查。日志为 `miri-compiler-boundary.log`。

- 完整 M0 通过：263,843 ms、0 hard failure、83 match / 9 debt，没有放宽比较规则。
- Debug/Release 各十个官方严格文件 × 三引擎通过，共 60 次执行；逐项检查正常
  退出、原始完成标记和空 stderr。保留 Rust literals 的原版 `pt_BR` locale 不可用跳过。
- CLI 各 18/18、REPL 各 8/8、bytecode 合同各 15/15 通过。后者包含 12 项固定
  C++ 共有合同和 3 项 Rust 扩展。上述官方/工具共 218 次执行的原始输出逐一验证，
  汇总在 `final-independent-validation.json`。
- 两个嵌套非法源码另经 bytecode Text 三种二进制共 6 次执行，全部精确匹配固定
  C++ 的 exit 3、空 stdout 和异常诊断；Rust JSON 扩展另 4 次执行均拒绝且不输出 dump。

完整 M1 基础门通过：284,275 ms、0 hard failure；fmt、Clippy、Debug/Release 全 targets、
doctests、rustdoc、audit 均完成，无跳过。普通测试清单 1,080 项，无 ignored，较前一
快照新增 compiler 17、CLI 5、stdlib boundary 2 项。`m1Complete=false` 仍然保留，
通过基础门不等于所有 M1 合同完成。七条 M1 步骤引用对应的六份唯一产物及索引已存入 `m1-step-artifacts/`，
避免后续覆盖默认 target 路径丢失本次证据；总核验见 `final-verification.json`。

## 输入身份与解释边界

最终 Cargo/配置/源码/fixtures/tools/CI 共 376 个输入保存在 `source-final.zip`，
清单 SHA-256 为 `b29389c486d4537440e68e10e655a975179d6a2c9453f695f3f079a94023f25a`。
测试从相同工作区构建，归档用于固定输入身份，不声称从 ZIP 重建。
Miri 启动时的清单为 `cfc495db8007d131e7341de3b1d1e618fc1522ff090627bfe50496389a1743e7`；
两份输入仅编译器 integration test 文件不同，Miri 实际使用的库、配置和 boundary
target 一致。验收结束后再次核对 376 个输入文件，变化数为 0，四个最终 Rust
二进制与独立运行的快照一致，完整 SHA 保存在 `binaries-final.json`。
全部证据集中在 `target/compiler-error-propagation-evidence/`。

固定 C++ 与 Rust 都可能跳过多余的 local 初始化表达式，例如
`local a=1,function() do 1=2 end end`；独立实测两者均返回 function。该共同限制
未被本次改变。因此本轮合同是传播实际进入 codegen 的嵌套错误，不能外推为
拒绝所有文本上含非法表达式的源码，也不能据此宣称已达到 stock Lua 的 parser 行为。

后续 Windows files 的具体差异已单独收口，见 [popen 整改](2026-09-12-popen-remediation.md)。
以下保留最初定位记录：受控 PATH 的九进程
探测已确认：固定 C++ 只对精确命令 `ls` 执行 Windows `where` 可用性检查，找不到
时在创建管道前抛出 `io.popen: command not available`；Rust 缺少该检查。
`ls `、shell 内建命令以及存在临时 `ls.cmd` 的情形已分开记录，stock 的非零退出
close 返回合同差异也单独保留。证据在 `target/files-popen-evidence/`；该早期探测时尚未修改 IO。
`string.dump`、binary chunk、
C API 和 native module 仍按既有 M3 合同设计推进，完整 M1/M2 尚未完成。
