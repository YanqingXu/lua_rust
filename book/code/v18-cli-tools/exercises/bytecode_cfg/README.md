# CFG 为什么按指令找 leader，不能按源码行画框？

本包只有一道 TODO：`starter.rs::leaders(&Proto) -> Result<Vec<usize>, String>`。
输入使用真实 `Proto/Instruction`；图的分块、出边与 DOT 格式由框架提供。先预测：
一次 jump 后面不一定会执行的指令，是否仍要切出基本块？同一源码行能否包含多个块？

从仓库根目录执行：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --manifest-path book/code/v18-cli-tools/Cargo.toml --example bytecode_cfg_problem_chain -- all
```

示例接受 `split|target|lines|real|all`，缺省 all；分别回放漏切分、错误跳转基址、
源码行分块、真实 compiler/public CFG/binary 对照。它不读取 starter，固定 faults
与 solution 不会随着学习者填写而改变。退出 0 只表示固定回放通过，不代表 TODO 完成。

## 为什么少切一个块会连边都消失

`fixtures::hidden_jump` 是六条手造指令，全部标为同一个源码行号：

```text
0 Move
1 Jump sBx=2       -> 1+1+2 = 4
2 Move             （entry 沿 jump 不会走到这里）
3 Return
4 Move
5 Return
```

固定错误 `faults::entry_and_targets` 只放入 entry 0 和 target 4，得到 `[0,4]`。
框架据此切出 `[0,4)`、`[4,6)`。框架只看块末指令发边，第一块末尾是 Return，
所以 pc1 的 jump 藏在块内部，整张图没有边。错误源头是 leader 发现，不能靠在边
生成阶段扫描块内每条指令来绕过。

正确 leaders 是 `[0,2,4]`，半开块为 `[0,2)`、`[2,4)`、`[4,6)`，出边恰为
`B0 -> B2 [jump]`。`[2,4)` 没有入边仍然存在；leader 发现不是可达性裁剪，也没有
保证这些手造指令具备可执行的寄存器/常量窗口。它们只用于静态 CFG 结构实验。

## TODO 的四项规则

1. 空 code 返回空 leaders；非空先放入 entry 0。
2. 对六种 jump，将 `pc + 1 + sBx` 放入 leaders，结果必须在 `[0, code.len())`。
3. jump 或没有 fallthrough 的指令后，将 `pc+1` 放入 leaders，但只在范围内时加入。
4. 用 BTreeSet 排序、去重，再返回 Vec；不要看 source lines 或嵌套 Proto。

每一条 opcode 都要解码；非法 opcode 必须报错，即使它不是当前块的末指令。
计算负 offset 时先用有符号算术，范围验证成功后才能转换成 usize。到 code.len()
也算越界；当前 renderer 没有为“代码尾外”建立隐式 exit 节点。

```diff
-Ok(if proto.code.is_empty() { vec![] } else { vec![0] })
+// nonempty: BTreeSet { 0 }
+// each instruction: validate opcode, add pc+1+sBx target for a jump
+// add in-range pc+1 after jump or no-fallthrough
+// return ordered unique starts
```

这里六种 jump 是 Jump、JumpIfFalse、JumpIfTrue、JumpIfNil、ForPrep、ForLoop。
只有 Jump、ForPrep、Return、TailCall 没有 fallthrough。三个条件 jump 和 ForLoop
都有 jump/next 两种可能；Return/TailCall 没有 jump，也不发 next，但它们后面的
指令仍可成为 leader。这个 checkpoint 的 Eq/Lt 等比较产生值，不能照搬其他字节码
格式中“比较指令本身跳过下一条”的约定。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_cfg_exercises leaders_split_control_flow_and_validate_instruction_targets -- --ignored --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_cfg_reference leaders_split_control_flow_and_validate_instruction_targets -- --exact
```

两条各运行一项。初始 learner 应失败；完成后同一个合同通过。合同覆盖前/后向跳转、
三种条件分支、ForPrep/ForLoop 的 fallthrough 区别、Return/TailCall、空 code，另验
invalid opcode、负 target、等于长度的 target 和更远越界。每例核对精确 starts、
半开 blocks、完整有标签 edges；手写 expected graph 渲染的完整 DOT 与公开
`lua_bytecode::cfg_dot` 逐字节一致。只检查包含 jump/next 关键词不算通过。

## 另外两个固定错误

`faults::pc_relative` 把 target 写成 `pc+sBx`。条件例的 pc1/sBx1 实际 target 为 3，
错误版只得到 leaders `[0,2]`，给定边框架会报 `pc 1 targets non-leader 3`。
修复为 `[0,2,3]` 后，精确边是 B0→B2 jump、B0→B1 next、B1→B2 next。

`faults::source_lines` 把源码行变化当分块依据。六条同一源码行的例子只剩 leader0，
又丢了 jump。反过来，三条直线 Move/Move/Return 若行号各异，会多分成三个块，
而规范 leaders 仍只有 `[0]`。行号服务诊断，不决定这套字节码的控制流边界；它可以
在块内变化，也可以在跨块时不变。

给定 `model::graph` 把相邻 starts 形成 `[start,next_start)`，最后一个 end 是 code
长度。只有每块最后一条指令决定出边：先 jump，再 next；普通 fallthrough 到本块
end，且 end 在 code 范围内才连。给定框架不执行字节码，不判断分支条件恒真/恒假，
不删除不可达块；同一个 Proto 的 `protos` 子函数也不递归生成图。参考另放入带非法
opcode 的 nested Proto，验证当前层的图仍正常，避免把这一局部图说成全程序 CFG。

## 真实 compiler 与 binary 如何对照

`real.rs` 编译两段正常 Lua，不执行它们：

```lua
if flag then return 1 end
return 2

-- 另一份独立 chunk
local total=0
for i=1,2 do total=total+i end
return total
```

测试为本快照手写 expected ranges/edges，再同时核对练习图与公开 cfg_dot 完整文本。
if 为 `[0,2),[2,5),[5,7),[7,8),[8,11),[11,12)`，共 4 条边；for 为
`[0,9),[9,14),[14,15),[15,19),[19,20)`，也有 4 条边。compiler 产生的 Close 和
隐式 Return 也完整保留，没有按源码表面形状猜 basic blocks 或删掉无入边 Return。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_cfg_reference real_compiler_if_and_for_match_exact_blocks_and_edges -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test bytecode_cfg_reference real_binary_cfg_matches_public_dot_for_compiled_sources -- --exact
```

真实 `lua_bytecode --format cfg -` 接收同一 source bytes，以 code0、空 stderr 和完整
stdout 字节与公开 cfg_dot 对照，分别为 426/478 bytes。integration 从 Cargo 提供的
lua_app 路径定位同 profile 的 lua_bytecode；示例从 current_exe 定位 sibling，均
复用共享 process runner，不从 PATH 选择别的解释器，不在示例里嵌套启动 cargo。
先 `cargo build --workspace --bins`，使用自定义 target-dir/Release 时构建与运行参数
保持一致。helper 只管理本轮无后代、有界输出的 exact child，不承诺通用进程树或
后代持 pipe 时的 join 上界；此测试也不替代 runtime trace。

默认 **7 项 reference、1 项 ignored learner**。理解题：为什么 Return 后面的块
没有入边还要保留？实现题：只完成 leaders hook，显式运行一项通过。调试题：比较
entry+target、错误 base、source line 三个故障各自怎样改变 blocks/edges。完成后
应能从每个指令 pc 手推 target、starts、半开区间和块末出边，区分静态 CFG、可达
分析与某次执行的 instruction trace。公开 viewer 当前只生成传入 Proto 的图。
