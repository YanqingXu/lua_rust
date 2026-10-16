# v1.0 · Lua 5.1 语言运行时兼容收口

<div class="lesson-meta"><span>核心增量：从“有实现”变为“有证据的发布合同”</span><span>6-crate workspace</span><span>checkpoint v1.0</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>让每个兼容结论都有可追溯证据</strong>
  <p>前置：v0.18 的语言、CLI 和观察协议。新知识：闭世界矩阵、双向引用校验、schema
  version、oracle provenance、差分子进程与 release gate。完成后你应能区分代表性通过、
  partial deviation 和被明确排除的版本边界。先把结构化 REPL 协议迁入本地快照，
  再用同一份原始字节合同验证两个独立 binary；随后从 string.format 的真实差异
  推导格式规格、参数检查与有界兼容证据；再从 string.find 的空匹配和失败后缀
  推导字节 atom、按路径解析与显式回退栈；最后用捕获槽的变化推导检查点恢复、
  持续计费和返回字节的所有权。</p>
</div>

v0.18 已经能运行脚本、模块和协程，也能观察 bytecode、CFG 与 trace。但功能数量不是兼容性证据：如果测试没有说明参照物、平台归一化规则和允许差异，“测试通过”仍可能只是与自己一致。本版完成三件事：**补齐核心源码语法缺口、冻结可观察协议、让每项兼容结论都能追溯到测试或偏差 ID**。

<a id="learning-predict"></a>
## 先预测再运行

判断以下证据各能证明什么：一个 golden output、五个 official-oracle fixtures、38 项语义
映射、完整官方测试套件。它们强度不同，不能互相冒充。再为 complete/partial/excluded
各写一行能力矩阵，检查是否需要 test evidence、deviation ID 或 future-version boundary。

再预测：v0.18 已能将两行 `return 40 +`、`2` 合成42，v1.0 是否自然拥有这个行为？
输入未闭合 block 后直接 EOF，退出码应该是多少？最后一个短字符串片段有没有 LF，
会不会改变分类？先保留这些预测，运行下方迁移反例再核对。

再写下 `string.format('%f', 1.5)`、`string.format('%04d', 7)` 和
`pcall(string.format, '%s', {})` 的预期。若本地测试把错误结果写成 expected，
测试全绿还能说明兼容吗？下面的问题四至六将把这个差异变成可执行规格。

再预测 `string.find('abc','',99,true)`、`string.find('axb','a.b')` 与
`string.find('aaab','a*ab')`。最后比较 `'z['` 和 `'a['` 在 `'abc'` 上的结果：
未闭集合总要立即报错吗？问题七至九从这些输入建立新的搜索状态。

再预测 `string.find('aaab','(a*)(a)b')` 的两个捕获：贪尽时第一槽已经关闭，第二槽
已经新建，失败后只截短Vec够不够？`()`与匹配到空字符串有什么区别？返回的NUL/
0xff捕获在下一次find或Session释放后由谁保存？问题十至十二继续沿失败轨迹演进。

<a id="learning-code-map"></a>
## 代码阅读地图

1. capabilities.tsv：闭世界列出功能、状态、证据和 deviation；
2. deviations.tsv：为 partial 项登记范围、处置和目标版本；
3. release tests：双向检查 capability↔deviation，不允许孤儿或假 complete；
4. versioned JSON/trace envelope：冻结 consumer 可观察协议；
5. Lua51Opcode audit：覆盖官方 ordinal，但明确不是 binary compatibility；
6. oracles.toml：固定版本、commit、URL 与 hash；
7. differential runner：三个真实进程、唯一归一化、timeout 与 lane 结果；
8. release gate：本地质量门与可选外部 oracle 证据分层报告。

本轮 REPL 微课另按调用顺序读：本地 `src/main.rs::repl` → `src/lib.rs::run_repl`
→ `Repl` → `Session::execute_multi` → 本地 `lua_compiler::compile` → VM。先看
compiler 返回的 kind，再看 controller 保留的 source/诊断，最后看 adapter 的 EOF；
`tests/repl_byte_contract.rs` 负责从进程外核对这条链。

格式化另沿 `Vm::invoke_string` → `string_format::format_arguments` → `format`
→ `parse_spec` → 各 renderer 阅读；`tests/string_format_contract.rs` 从实际 binary
验固定字节，再单独检查本地错误 kind、参数号与分类。

find 则读 `StringFunction::Find` adapter → `string_find::find_captures` 的 init/plain
分流 → `match_at/capture_control` → 到达的 `parse_atom/Atom::matches` →
`greedy_end/retry` → `finish_match` → adapter物化捕获。旧`find`仍提供整体Range投影。
adapter拥有本次参数字节副本，matcher借用它们，Set借用pattern切片；FindMatch拥有
范围元数据，返回的ByteString另外拥有捕获bytes，没有延长输入借用或heap对象生命周期。

本版阅读顺序从证据到实现：先问“这个结论由什么证明”，再进入 parser/opcode/tool 代码。
测试存在并不自动成为证据；只有覆盖对象、参照物和允许归一化都明确时，才能支撑对应行。

## 先定义“完成”

v1.0 使用闭世界模型管理发布范围：

```text
能力矩阵中的每一行
├── complete ──> 必须指向自动测试，不得绑定偏差
├── partial  ──> 必须绑定已登记偏差
└── excluded ──> 必须绑定 v1.1+ 的明确边界

偏差注册表中的每个 ID ──> 必须反向出现在能力矩阵
```

这两个方向都由 `tests/lua51_release.rs` 检查。删除能力引用、拼错 ID、给 `complete` 项偷偷绑定例外，都会让普通 `cargo test` 失败。能力真相源位于：

- `compatibility/capabilities.tsv`：功能、状态、测试证据与偏差 ID；
- `compatibility/deviations.tsv`：范围、处置状态、目标版本和精确描述；
- `compatibility/oracles.toml`：外部参照物的版本、URL、commit 与 SHA-256；
- `compatibility/differential-baseline.json`：已经实际得到的双差分结果。

选择 TSV 是为了让 Rust 测试无需第三方解析库也能执行 closed-world 校验；选择 TOML/JSON 保存 provenance 与结果，是因为字段层级更自然，也方便外部工具读取。

## 补齐 Lua 的三种调用形式

Lua 5.1 的函数参数不只有括号形式：

```lua
consume("lua")
consume "lua"
consume { answer = 42 }
```

v0.18 的 `postfix` parser 只在遇到 `(` 时生成 `Call`。v1.0 把 `(`、字符串 token 和 `{` 都视为参数起点；table constructor 和 string 会包装成单一实参。语句入口也接受括号 prefix expression，因此 `(function() ... end)()` 可以作为独立调用语句。

这处改动揭示一个 Lua 边界：换行不是 statement terminator。下面的 `(` 可能成为上一行表达式的调用后缀，因此需要分号消除歧义：

```lua
local result = 0;
(function() result = 42 end)()
```

端到端测试同时执行三种参数形式、括号调用语句和多返回，证明改动最终穿过 Codegen 与 VM，而不只是 AST snapshot 正确。

## 问题一：前一版修好了，独立快照为什么仍然失败？

### 先重现：版本号不会替我们迁移代码

迁移前的 v1.0 和 v0.18 各有自己的 compiler、controller 与 main。旧 v1.0 的
`is_incomplete` 在 message 中搜索 `expected 'end'`、`unexpected end`、
`unterminated`；`return 40 +` 实际得到 `expected an expression`，漏判后清空
source，下一行 `2` 也无法接上。对旧提交 `7776fd4` 的真实 binary 输入
`return 40 +\n2\n=42\n`，结果是两次诊断，只有最后的新 chunk 输出42。

本地固定反例仍能复现这个决策，不需要下载历史 binary：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example repl_migration_problem_chain -- all
```

`faults` 重放关键词漏判、静默 EOF、clone 重复诊断；`migration` 调用当前本地
compiler/Repl/driver。缺省 `all` 共12条不同的 `[ok]`，只运行固定参考；它没有
替你完成 learner，也不替代完整进程双流与退出码合同。

### 最小迁移：先迁移分类，再迁移消费者

```diff
-Err(AppError::Compile(error)) if is_incomplete(&error) => continue,
+Err(AppError::Compile(error)) if error.kind == CompileErrorKind::NeedMoreInput => {
+    self.pending = Some(error);
+    Ok(ReplEvent::NeedMoreInput)
+}
```

先给本地 `CompileError` 增加 kind，再迁移具体词法/语法错误构造点，最后迁移
consumer。原 `compile(&[u8]) -> Result<Proto, CompileError>` 与 Display 保留；
Rust 调用者若自己构造错误，需要补 kind 字段。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_compiler/src/lib.rs:repl_compile_error}}
```

EOF 处还期待表达式、名称或闭合符时可返回 NeedMoreInput；错误 token、畸形数字、
非法 escape、未转义的短串换行和 codegen 错误仍是 Invalid。延迟 unfinished
literal 的词法错误，才能让更早已确定的普通语法错误优先返回。这是**具体构造点的
局部提示**，不是“任意前缀总能补成合法程序”的证明；每次仍重编译完整 buffer。
完整分类推导见[上一版的问题十一](v18-cli-tools.md)。

### 保留本版增量：不能整份覆盖 parser

v1.0 已比 v0.18 多了括号 prefix expression，以及 string/table call sugar。
迁移要把新的分类接入这些本地路径，保留它们接受的合法程序：

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_compiler/src/lib.rs:repl_v100_call_arguments}}
```

例如 `consume { answer =` 可以等待值，而 `consume { answer = )` 必须立即拒绝；
`consume 'unfinished` 无 LF 可以等待闭合，有真实 LF 则不能继续短字符串。
`(function() return 42 end)()` 仍能作为语句执行。这样读者能看见“迁移协议”与
“保留版本增量”分别由哪些路径承担。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml -p lua_compiler tests::v100_prefix_and_call_sugar_keep_structured_error_kinds -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml -p lua_compiler tests::v100_prefix_and_call_sugar_still_complete_valid_protos -- --exact
```

第一项逐个核对 unfinished/Invalid，第二项把分段 source 补齐并检查成功 Proto；
真实 controller 与 CLI 回归再验证执行结果。只有“能编译”不足以证明执行次数与状态。

## 问题二：谁保存输入，谁消费 EOF，谁保留 VM？

### 一个迁移，三个 owner 边界

![v1.0 本地迁移、输入状态与 EOF 所有权](../images/v100-repl-migration.svg)

`Session` 拥有 VM；`Repl<'a>` 借用同一个 `&'a mut Session`，自己拥有 source
与 `Option<CompileError>`；`run_repl` 借用输入输出，识别传输事件。main 只接线
并把最终错误转换为退出码。不能为复用 controller 把 main 覆盖回旧版：本版
`Action::Help` 仍参与 action 顺序，`-e/-l/-v/-i`、arg table 与 v1.0 banner 都要保留。

每次提交先编译整个 source；只有编译成功才执行那个 Proto 一次。未完成时保留
source 与最近一次编译诊断，**前缀零执行**；成功或普通错误返回后清当前输入，Session 不重建，
已经发生的 runtime 副作用不回滚。调用者用公开 `submit` 提交不含末尾 LF 的完整物理行；driver
内部用 `submit_fragment` 保留末片段是否真的带 LF。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/src/lib.rs:repl_structured_submit}}
```

### EOF 不是“再试一次编译”

旧 v1.0 的 `read_line -> Ok(0)` 直接 break。输入 `if true then\nprint('never')\n`
后 EOF，真实旧 binary 无诊断且 exit0，未执行的输入就这样丢了。正确路径消费
已经保存的诊断：

```diff
-if input.read_line(&mut line)? == 0 { break; }
+if count == 0 { return repl.finish_eof(); }

-// 丢弃未完成的 source，正常返回
+self.source.clear();
+match self.pending.take() {
+    Some(error) => Err(AppError::Compile(error)),
+    None => Ok(()),
+}
```

`take()` 将 Some 移出并留下 None；clone 会让第二次 finish 仍报同一诊断。EOF
不会重编译或执行；pending EOF 将原错误交给顶层打印一次并 exit1，idle EOF 为0。
此前普通行错误已经报告、buffer 已清时，最后仍可走 idle EOF。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/src/lib.rs:repl_structured_eof}}
```

只有 `read_line -> Ok(0)` 是 EOF。正数是片段，有 LF 才移除 LF 与可选 CR，再按
有换行提交；Err 是宿主错误，不能换成旧 pending 编译诊断。完整无 LF `=42`
照常执行；`return 'broken` 无 LF 是 NeedMoreInput 后 pending EOF=1，加 LF 是
短串非法换行，报告并清 buffer 后 idle EOF=0。旧 v1.0 在无 LF 情况也会合成换行，
因此它原先报告 `newline in short string` 后 exit0；不能把这条历史路径说成静默等待。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/src/lib.rs:repl_structured_transport}}
```

input 使用 UTF-8 String，非 UTF-8 属于 Host 错误；泛型 writer 承接提示符与返回值，
Lua `print` 仍走进程 stdout。它不是全局输出拦截器，也没有实现增量 parser。

### 验收练习：把原诊断移动一次

只改[迁移练习包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/exercises/repl_migration/README.md)
的 `starter.rs::finish(&mut PendingInput)`，保留其他文件、合同与 ignore。
PendingInput 用本地真实 compiler 生成诊断，是所有权教学投影；实际 controller
另验，不能用投影通过冒充 VM 行为通过。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/repl_migration/starter.rs:repl_migration_starter}}
```

合同要求 idle 成功、pending 返回原 kind/位置/message 一次、source 与 Option 清空，
约束禁止重编译/执行。hook 不接收 Session，不代表 Rust 禁止调用任何全局函数。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_exercises migrated_eof_consumes_original_diagnostic_once -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_migration_reference migrated_eof_consumes_original_diagnostic_once -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_input_protocol pending_chunks_execute_once_and_keep_the_same_session -- --exact
```

第一条初始应失败，补完后通过；第二条固定参考通过不表示 learner 完成。第三条
沿真实 Session 验证前缀不运行、成功恰一次和状态保留；包内还有普通错误恢复、I/O
Err、binary 返回值与 shorthand 边界验收。Release 加 `--release` 重验同一规格。

## 问题三：怎样复用合同，又让两个快照独立构建？

### 相同输出不一定正确

若两边都静默丢掉 pending EOF，只比较两份实际输出仍会通过。expected 必须来自
固定 oracle。仓库的 `tools/repl_byte_contracts/cases.tsv` 保存26条场景：前21条
沿用 v0.18 字节验收，再补5条 CRLF 与末片段无 LF 边界；每行固定 argv、stdin、
退出码、banner 策略、stdout body、stderr，bytes 用小写 hex 保存。

![共享固定合同、独立运行和证据归档](../images/v100-repl-byte-contract.svg)

```diff
-// 只比较两个实际 stdout，或 trim/归一化后检查“有42”
-assert_eq!(v18_output, v100_output);
+assert_eq!(actual.status.code(), Some(case.exit));
+assert_eq!(actual.stdout, case.stdout(version));
+assert_eq!(actual.stderr, case.stderr);
```

两个目录各保存完整 fixture、测试 wrapper 和安全 process runner；没有跨版本
`include!`、path dependency 或读取相邻 target。仓库 gate 检查三份 corpus 与
两份测试/runner 逐 byte 相同，防止复制后漂移；每个快照的本地测试不需要仓库 gate
也能运行。版本差异只有显式的 banner 映射：

```rust,ignore
{{#include ../../code/v100-lua51-runtime/tests/support/repl_byte_contract.rs:repl_contract_banner}}
```

不能 trim 输出、丢弃 stderr、将 0xff 有损解码、改 CRLF 或忽略尾 LF。NUL、提示符、
诊断次数都在 expected bytes 内。runner 写完关闭 stdin，同时 drain 两流，deadline
后 kill/wait 并 join；这是有界、无后代 fixture 的合同，不外推任意进程树治理。

```powershell
py -3 tools/check_repl_byte_contracts.py --check
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test repl_byte_contract real_binary_matches_all_shared_raw_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test repl_byte_contract real_binary_matches_all_shared_raw_byte_cases -- --exact
```

同一测试在两个独立 binary 上各跑全部26条；完整 stdin/stdout/stderr 与 exit 各自
对固定 oracle。独立 Python runner 还能另启真实进程、归档 actual 与 expected 双流、
metadata、corpus/harness/binary SHA，运行目录独占，拒绝覆盖。构建及传路径命令见
[共享合同说明](https://github.com/YanqingXu/lua_rust/blob/main/tools/repl_byte_contracts/README.md)。本合同不作通用文本
normalize；下文语言 differential runner 的 CRLF 规则属于另一组明确声明的合同。

先故意改坏一份副本，应被身份 gate 拒绝；再设想两个只输出正确 banner、固定 idle
提示符的 binary，它们相互一致，仍应被固定 oracle 拒绝。发布结论需要这些失败路径
也能观察，不能只看两边 exit0 或示例打印 `[ok]`。这26条证明本地 REPL 策略与列明
边界，不代表官方 Lua 全部 TTY 行为；signal、编辑器、增量解析仍要另立规格。

## 问题四：`%f` 有输出，为什么仍然不兼容？

### 从一个错误的绿色测试开始

迁移前的真实 v1.0 用 `f64::to_string()` 实现 `%f`。旧 `tests/stdlib.rs` 还将
`'%s:%d:%f:%%'` 的预期写成 `x:7:1.5:%`，因此测试通过恰好固定了偏差。
对提交 `74fe6ca` 的 binary 和官方 Lua 5.1.5 实际运行，可观察到：

| 输入 | 修改前 v1.0 | 官方 Lua 5.1.5 |
| --- | --- | --- |
| `string.format('%f', 1.5)` | `1.5` | `1.500000` |
| `string.format('%04d', 7)` | 报 unsupported `%0` | `0007` |
| `string.format('%.2f', 1.5)` | 报 unsupported `%.` | `1.50` |
| `pcall(string.format, '%s', {})` | `true` | `false` |

先用固定反例对照当前真实 VM：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example string_format_problem_chain -- all
```

缺省 `all` 共12条不同 `[ok]`：`faults` 四条重放通用显示、单字节规格、宽泛
stringify 和过早推进游标，`format` 八条调用本地 parser/VM。固定参考与反例的
通过不代表 learner 完成，也不代替后面的官方进程字节对照。

### 最小变化：精度成为独立输入

```diff
-output.extend(number.to_string().as_bytes());
+let precision = usize::from(spec.precision.unwrap_or(6));
+let digits = format!("{:.*}", precision, number.abs()).into_bytes();
+// 符号和宽度补齐由独立步骤处理。
```

通用显示与格式协议有不同目标。`%f` 没有 precision 时默认6；`%.2f` 指定2，
`%.f` 指定0。后两者不能被“去掉多余零”实现。`Option<u8>` 把未提供与显式0
区分开，renderer 才能选对默认规则。旧测试的预期也必须随已确认的官方合同修正。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_format.rs:format_render_numbers}}
```

数值先校验，再分离符号与绝对值数字；零宽度补齐应位于符号之后。`+` 压过空格，
左对齐压过零补齐；整数显式 precision 压过零宽度补齐。`%#.0f` 在零小数位时仍
保留小数点，`-0.0` 的符号也不能在取绝对值后丢掉。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml -p lua_vm string_format::tests::fixed_floats_have_six_default_digits_and_sign_aware_zero_padding -- --exact
```

该测试同时观察默认6位、符号后补零、`#`、左对齐与负零，并明确拒绝非有限值。
本次 `d/i` 有界支持共同的 signed i32 范围；超范围返回错误，C locale、宿主浮点
舍入边缘与其它转换仍登记 partial，不把几组普通数值外推为完整 printf。

## 问题五：为什么 `%` 后不能只读取一个字符？

### 让输入迫使数据结构出现

`%+08.2f` 的第一个字符 `+` 是 flag，第二个 `0` 也是 flag，`8` 才是 width；
`.` 后的 `2` 是 precision，最后的 `f` 是 conversion。单字符 match 把这些层次
混成一个“未知转换”，读者无法逐步补齐。先把五个 flag 的语义与可选数值存下来：

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_format.rs:format_spec}}
```

![格式规格解析与渲染优先级](../images/v100-format-spec.svg)

parser 共享借用 `&[u8]`，局部 cursor 按 flags → width → precision → conversion
前进；成功返回 `(FormatSpec, next)`，由 dispatcher 提交 next。失败走 Result，
不按错误文字猜测已经消费多少，也不把某个数字默认成0继续运行。

```diff
-let specifier = format[cursor];
-cursor += 1;
+let (spec, next) = parse_spec(format, cursor)?;
+// 用 spec 渲染成功后，再将 format cursor 设为 next。
```

这里的 start 紧接 `%`；`%%` 已在 dispatcher 提前处理，不进入规格 parser。
Lua 5.1 的 scanformat 有两个值得亲自预测的限制：flag **字符数**最多5个，
`%++d` 可以接受，第6个才报 repeated flags；width/precision 最多两位，第三位
报错。两位十进制累积不会超出 u8，范围由语法本身约束。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_format.rs:format_parse_digits}}
```

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_format.rs:format_parse_spec}}
```

`%100d`、`%.100f`、`%*d`、末尾 `%` 和未知转换分别触发相应失败路径；`%.f`
则合法地返回 `Some(0)`。这里继续只支持已有 `s/d/i/f/q/%%`，不会因为 scanner
能识别规格结构，就声称已实现 `%x/%e/%g/%c` 等转换。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference actual_parser_checks_the_whole_bounded_specification -- --exact
```

测试直接借用真实 `parse_spec`，检查字段、next、重复 flag 与长度边界；练习没有
再写一个简化 parser。规则依据可查[Lua 5.1.5 的 scanformat/str_format 源码](https://www.lua.org/source/5.1/lstrlib.c.html#scanformat)。

## 问题六：参数能显示出来，为什么仍必须报错？

### 两个游标，各有成功与失败边界

`%s/%q` 接受 string 或 number，`%d/%i/%f` 接受 number 或可转换的数字字符串；
table、boolean 和 nil 不因通用显示能生成文字而合法。第一参数格式串也要检查
缺参与类型，数字格式串使用同一 number-to-string 路径，不绕回 Rust 通用显示。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_format.rs:format_arguments}}
```

![格式游标、参数游标与错误返回](../images/v100-format-arguments.svg)

普通字节只推进格式游标，`%%` 输出一个 `%` 且不拿参数。其它转换先检查下一项
是否存在，再 parse 规格、检查类型并渲染。因此 `string.format('%100d')` 先报
缺参数 #2，提供参数后才报 width 太长；每个转换的诊断参数号还要包含格式串 #1。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_format.rs:format_dispatch}}
```

局部 `Vec<u8>` 累积的前缀不会在后续错误时成为成功返回值。Err 经 VM 的 Type
错误传到 pcall 或 CLI；format 没有中途输出字符串。此前 chunk 的 globals/I/O
副作用仍已发生，这不是整个 VM 的事务回滚。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract format_type_errors_keep_kind_and_argument_number -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract missing_values_take_priority_before_format_parsing -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract invalid_format_errors_do_not_become_argument_errors -- --exact
```

三条命令分别检查 Type + 参数号、缺参优先级、格式错误分类。不能把任何异常都
吞成“预期失败”就算通过，也不要求不同引擎的 traceback/源码路径完全相同。

### bytes 的边界也要由反例决定

Lua 5.1.5 的短 `%s` 路径按 C 字符串处理 NUL；没有 precision 且输入长度至少100
时，源码走保留原 bytes 的 fast path，并绕过 width。显式 precision 则回到短路径。
`%q` 转义 NUL、引号、反斜杠和换行，供 Lua 重新读入；实际 LF 被编码为反斜杠后
跟 LF（`5c0a`），不能仅因为 `5c6e` 也可回读成 LF，就把两份输出称为逐字节一致。

这些实现细节连同数字字符串转换，以[官方格式说明](https://www.lua.org/manual/5.1/manual.html#pdf-string.format)
和固定 5.1.5 源码、实际 binary 对照。固定 byte fixture、真实输出和 `loadstring`
回读都分别验证，不用宽泛“支持二进制字符串”替代每个路径的合同。

### 验收练习：先验证，再提交参数游标

只改[格式化练习包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/exercises/string_format/README.md)
的 `starter.rs::consume`。spec 来自真实 parser，values 共享借用完整参数切片，
index0 是格式串，cursor 从1开始。成功返回一个 Value clone 并推进一项；缺参或
类型错误返回 HostError，cursor 保持原值。Value clone 不深拷贝 Lua heap 对象。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_format/starter.rs:string_format_starter}}
```

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_format/solution.rs:string_format_solution}}
```

这是参数检查与游标提交的教学投影，复用真实 Args；它不替代真实 formatter 的
数字范围、数字字符串边缘、NUL 和渲染。`?` 在失败处立即返回，只有全部检查成功
才改 cursor。fixed eager fault 提前推进，让缺参/类型错误也消耗了位置。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_exercises argument_cursor_commits_only_after_validation -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_reference argument_cursor_commits_only_after_validation -- --exact
```

初始 learner 应失败；只补 hook 后通过。默认 ignored、9项固定参考和12条示例
marker 不能替代 learner 完成；Release 加 `--release` 验同一边界。

### 官方 Lua 5.1.5 字节合同

固定 TSV 和独立 Lua driver 共57个 case：数字默认精度、flags/宽度/精度、参数
消费、缺参/非法规格、类型、首参数 coercion、短串 NUL/长串 fast path、q bytes
与回读。预期手写固定，driver 不读取预期，不用 string.format 编码输出。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/tests/string_format_contract.rs:format_binary_contract}}
```

Windows C stdout 会把 LF 转成 CRLF；要观察 q 的内部 `5c0a`，先在 **Lua 层**
把结果 bytes 编成 ASCII hex，再 `io.write('ok|...|after')`。错误 case 输出
`error|after`，本地三组诊断测试另核类别和参数号。Python/Rust 捕获完整双流与
exit，不 trim、不替换 CRLF，也不用 formatter 自己生成 expected。未知 case、
driver 编译失败或宿主故障不能作为合法 error case 通过。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_format_contract real_binary_matches_fifty_seven_fixed_format_byte_cases -- --exact
py -3 book/code/v100-lua51-runtime/tools/check_string_format_contract.py --check
```

`--check` 只检查 schema/driver 一致，明确打印 candidate/official NOT_RUN。实际
对照必须同时给 candidate binary、official binary 与固定源码归档：

```powershell
py -3 book/code/v100-lua51-runtime/tools/check_string_format_contract.py --candidate-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --official-bin target/oracles/lua-5.1.5/build/Release/lua51.exe --official-archive target/oracles/lua-5.1.5/lua-5.1.5.tar.gz --output target/string-format-review
```

先 `cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app`
构建所选 profile；自定义 target/triple 时按实际产物改路径，Release 使用对应 binary。
runner 保存唯一 run/case 的实际/预期原始双流、状态、版本与 corpus/source/binary
SHA；超时后 kill/wait，缺 oracle 或归档失败显式失败。字段与复现边界见
[官方合同说明](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/tools/string_format_contract/README.md)。

本次能力矩阵新增 format 子集证据，byte-string 与 format 行仍为 **partial**，
继续引用 `LUA51-D005`。旧2026-08-21双 oracle baseline保持原版本归属；本轮官方
format证据单独归档，不把已有转换子集通过升级为完整 string library。

## 问题七：没有找到，和找到了零个字节，是一回事吗？

### 先对照当前接口实际承诺了什么

旧 v1.0 的 find 总是字面查找，未读取第四参数 plain；init超过末端就直接返回nil。
原 stdlib测试主要覆盖 `plain=true` 的字面成功，无法暴露默认pattern尚未实现。
从提交 `5347c60` 的冻结binary与官方5.1.5真实运行，可以观察到：

| 输入 | 修改前 v1.0 | 官方 Lua 5.1.5 |
| --- | --- | --- |
| `string.find('a.b','.')` | `2,2` | `1,1` |
| `string.find('abc12','%d+')` | `nil` | `4,5` |
| `string.find('aaab','a*ab')` | `nil` | `1,4` |
| `string.find('abc','',99)` | `nil` | `4,3` |

从仓库根运行反例与真实parser/VM对照：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example string_find_problem_chain -- all
```

`faults|find|all` 缺省all，共13条不同 `[ok]`。其中 literal fault 与大init直接nil
复现旧策略；另有强制忽略plain、贪尽不退、eager检查后缀等固定设计反例。忽略plain
的fault强制走新pattern路径，不能把它说成旧binary的逐行实现。固定参考和真实进程
合同各自证明不同层次，marker不代表TODO或官方验收已完成。

### 先分流、再解释结果

plain按Lua truthiness解释，数字0也为真；false/nil才为假。plain为真时完整pattern
按字节查找；默认模式还会扫描pattern首NUL前的C-prefix，若无SPECIALS，也进入
完整长度的字面路径。只有需要pattern解释时，才把该C-prefix交给matcher。

```diff
-if start > subject.len() { return Ok(None); }
-// Every request does a literal windows search.
+let start = normalize_init(init, subject.len());
+// plain truthiness / no magic → full-byte literal search
+// otherwise → match the pattern C-prefix, with subject bytes kept intact
```

这段是流程差异；实际init代码要处理负数与极端i64，不能把超界算术变成panic。
归一化的offset总在 `0..=len`，包括末端。空pattern在末端仍匹配成功。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_init_and_branch}}
```

![init、plain与NUL分流](../images/v100-find-routing.svg)

Rust 用 `Result<Option<Range<usize>>, FindError>`：Err是错误；Ok(None)是未命中；
Ok(Some(start..end))是成功，start等于end也合法。半开range映射到Lua闭区间为
`(start+1,end)`：`Some(3..3)` 是 `(4,3)`，空subject上的 `a*` 是 `(1,0)`。
因此不能用 `range.is_empty()` 或整数0代表“没找到”，还要核对返回元数。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/lib.rs:string_find_vm_adapter}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract plain_truthiness_init_empty_ranges_and_c_prefixes_remain_distinct -- --exact
```

### NUL 在主体和模式中，不走同一条规则

主体始终按完整bytes保留；`.` 能吃一个NUL或0xff，`%z`明确匹配NUL。pattern本身
则继承5.1.5的分流边界：`a\0x` 中找 `\0.`，自动plain按完整pattern查找而未命中；
`ab` 中找 `.\0x`，matcher只看`.`而返回 `(1,1)`。显式plain=true始终看完整pattern。
图中的 `\0` 表示一个NUL byte，复现脚本用string.char(0)构造，不传反斜杠字符。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_entry}}
```

分流与init依据[官方5.1.5 str_find_aux](https://www.lua.org/source/5.1/lstrlib.c.html#str_find_aux)，
固定fixture分别验主体、模式与显式plain，不以“用了byte slice”外推所有NUL行为。

## 问题八：为什么不能先把整个 pattern 验证一遍？

### 让错误时机约束 parser 的形状

`string.find('abc','z[')` 返回nil，因为各候选起点的z都失败，尚未到达未闭集合；
`string.find('abc','a[')` 才在a成功后报缺少`]`。全pattern eager parser会把两者
都立即拒绝，虽然诊断“看起来更早”，却改变了可观察行为。

```diff
-let ast = parse_entire_pattern(pattern)?;
-match_ast(ast, subject)
+// Only parse the atom reached by the current match path.
+let (atom, after_atom) = parse_atom(pattern, pattern_offset)?;
+// A failed atom does not inspect its unreached suffix.
```

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_atom_and_error}}
```

Atom分为Literal、Any、Class、Set；Set共享借用原pattern，不能比原bytes活得更久。
转义 `%.` 通过Class的fallback匹配字面点号；`%d/%D/%z`分别表示数字、补集与NUL。
parser只返回当前atom及其后位置，`*`/`+`由matcher读取，语法检查沿实际到达路径发生。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_parse_atom}}
```

![字节atom与错误到达时机](../images/v100-find-atoms.svg)

这不是Unicode正则。当前类按ASCII/C-locale解释，`%w`不包含下划线；5.1.5未定义
为字符类的 `%g/%G` 走字面fallback。集合支持范围、取反和escape，集合内的`%f`
不是frontier。`[]`也不是空集合：第一个`]`是集合内容，还需要闭合`]`，因此
`[]`报错而`[]]`能匹配字面`]`。locale差异和标准未定义的range/class混合不外推。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_ascii_class}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference actual_atom_parser_borrows_sets_and_matches_raw_bytes -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract find_errors_keep_argument_categories_and_only_validate_reached_atoms -- --exact
```

第一项借真实parser核atom、游标与NUL/ff匹配；第二项检查Type、参数号、具体malformed
类别及未到达后缀不报错。Invalid、Unsupported和Limit分别记录：未实现backreference、
`%b/%f`、optional/non-greedy量词不能静默按字面或统一返回nil。
转义、集合和普通字面上下文仍按各自规则；首`^`/末`$`的小范围支持也有单独回归。

## 问题九：贪婪匹配为什么必须保存“退一步”的状态？

### 后缀失败，也是推动设计的输入

`aaab` 匹配 `a*ab`：a*先吃三个a，从位置3试后缀ab，只剩b，失败；退到位置2
后剩ab，才成功。只调用 `try_suffix(greedy_end)` 会把合法匹配拒绝。`+`还要求
至少一次：`ab` 能匹配 `a*ab`，不能退到零次让 `a+ab` 也成功。

```diff
-return try_suffix(greedy_end);
+// Try the greedy end first; retain progressively shorter alternatives.
+save_retry(minimum, greedy_end - 1, suffix_pattern_position);
+// On suffix failure, resume from the next saved (subject, pattern) pair.
```

实际push只有在end大于minimum时才创建frame，不在无候选时计算end-1。每个frame
存下界、下一subject位置与后缀pattern位置；嵌套重复用显式Vec栈，没有宿主递归。
下方当前源码还带capture检查点和计费；本问先只跟踪位置，问题十一再推导这些字段。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_retry_state}}
```

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_greedy_candidates}}
```

![贪尽、失败与较短候选](../images/v100-find-backtrack.svg)

此图记录上一工作包的分流与回退边界；图末尚未纳入的capture在问题十至十二扩展，
不表示当前matcher会拒绝本轮已经覆盖的捕获。

步骤与frame深度有独立guard，默认100000步骤、128层。预算在同次搜索的候选间
共用，耗尽返回Limit，经adapter保留VmErrorKind::Limit；不是匹配失败，也不保证
精确wall-clock时限。字面fast path另走库的slice查找，不受matcher预算计量。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract candidate_work_limits_are_errors_and_leave_the_session_usable -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract unsupported_constructs_are_explicit_partial_errors_in_the_candidate -- --exact
```

用小limits确定性验步骤/栈深guard，再从真实VM触发默认Limit、读取同Session已保存
的全局变量，确认错误后仍能继续。Unsupported测试属于候选有界策略，不能把它
加入官方成功case中后宣称两边结果相等。

### 验收练习：下一次应提出哪个位置？

只改[find练习包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/exercises/string_find/README.md)
的 `starter.rs::next_position`。Retry投影保存 consumed range、minimum与next Option，
hook每次只提出一个后缀起点，不判断后缀。`take()`消费当前候选，checked算术计算
下一项；Some(0)合法，只有None表示耗尽，`+`不能低于至少一次的下界。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_find/starter.rs:string_find_starter}}
```

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_find/solution.rs:string_find_solution}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_exercises retry_positions_keep_greedy_order_and_the_repetition_minimum -- --ignored --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_reference retry_positions_keep_greedy_order_and_the_repetition_minimum -- --exact
```

初始learner应失败，补hook后通过；9参考和all13条marker不表示学生已完成。真实
atom和VM probes另验完整匹配、late error、坐标及恢复，练习没有第二套matcher。
Release对同样run/test加`--release`复核。

### 官方 Lua 5.1.5 的75场景字节合同

手写固定TSV保存返回元数与每项坐标/单nil；独立driver按id调用find，用select计数
而不丢掉nil。输出如 `ok|2|n:4|n:3|after`、`ok|1|nil|after` 或 `error|after`；
编码不依赖find或format生成预期，不输出换行。捕获完整stdout/stderr/exit，原始
bytes不trim、不normalize，区分返回一个nil与空匹配的两个坐标。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/tests/string_find_contract.rs:find_binary_contract}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_contract real_binary_matches_seventy_five_fixed_find_byte_cases -- --exact
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --check
```

本地测试无需外部Lua；`--check`只检查schema/driver，明确NOT_RUN。真实官方对照
要求同时提供两端binary与固定source tar，示例Windows路径按实际target产物调整：

```powershell
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --candidate-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --official-bin target/oracles/lua-5.1.5/build/Release/lua51.exe --official-archive target/oracles/lua-5.1.5/lua-5.1.5.tar.gz --output target/string-find-review
```

runner独占run/case目录，保存实际/预期双流、元数、状态、版本、输入与binary/source
SHA；缺文件、超时或归档失败显式失败。错误case用pcall稳定输出，Rust另核错误
类别、参数号与到达时机；未知id/driver编译错误在Lua `pcall` 之外，不能冒充合法pattern错误。
详细边界见[find官方合同](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/tools/string_find_contract/README.md)。

上一工作包的byte-string、format、find三行继续保持 **partial**，共同
引用 `LUA51-D005`。backreference、balanced/frontier、optional/non-greedy、
gmatch/gsub、locale与资源边界继续登记，既有format57条、REPL26条和旧语言双oracle
基线各保留原覆盖归属，不把find子集称为完整Lua pattern。

## 问题十：为什么整体匹配成功，还没有完整表达 find 的结果？

### 从当前 binary 真正拒绝的输入开始

上一包只收口整体匹配范围，捕获会明确返回Unsupported。冻结`db3e55f`实现的实际
Debug binary，对照官方5.1.5再运行，下列输入从错误变成了本轮要实现的规格：

| Lua调用 | 修改前 | 官方5.1.5的返回值 |
| --- | --- | --- |
| `string.find('id=42','(%a+)=(%d+)')` | error | `1,5,'id','42'` |
| `string.find('ab','(a*)ab')` | error | `1,2,''` |
| `string.find('ab','()a()')` | error | `1,1,1,2` |
| `string.find('aaab','(a*)(a)b')` | error | `1,4,'aa','a'` |

从仓库根运行固定设计反例与真实matcher/VM观察：

```powershell
cargo run --quiet --manifest-path book/code/v100-lua51-runtime/Cargo.toml --example string_find_captures_problem_chain -- all
```

`faults|captures|all`缺省all，分别3/9/12条不同`[ok]`。truncate、clear与退还工作
计数是固定设计fault；修改前binary是明确不支持capture，不能说成它曾使用这些错误
算法。先写预期，再区分“旧实现缺能力”和“一个拟议实现为什么还不正确”。

### 第一步：把工作状态和成功结果分开

一个`Range<usize>`只能表示整体范围。它装不下多个捕获，也无法区分空字符串和
位置。捕获编号按左括号出现顺序，而不是关闭顺序；`((a)b)`返回外层ab，再返回内层a。

```diff
-Result<Option<Range<usize>>, FindError>
+Result<Option<FindMatch>, FindError>
+// FindMatch { span, captures: Vec<Capture> }
+// working: Open(start) / Closed(range) / Position(offset)
+// success: Bytes(range) / Position(offset)
```

这是新增`find_captures/find_captures_with_limits`的返回协议。旧`find/find_with_limits`
仍投影整体Range，供上一练习使用，但先完成捕获验证；不能借投影绕过unfinished错误。
读真实类型，确认哪个Vec拥有状态，哪些字段只是索引：

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_capture_values}}
```

`Open(0)`表示尚未关闭，不能假造一个终点0；`Closed(0..0)`是成功的空字符串；
`Position(0)`则对应Lua数字1。它们不能用一个长度为0的range互相替代。

### 第二步：控制字节按实际执行路径改变槽位

先处理`(`/`)`控制，再把其它字节交给atom parser。普通`(`添加Open，紧接`)`的
`()`直接添加Position；右括号逆向寻找最近的Open，跨过已关闭槽和位置槽。
公开`parse_atom`只解析atom，本身仍不解释捕获；完整matcher在它之前分派控制字节。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_capture_control}}
```

![捕获工作状态与结果状态](../images/v100-capture-states.svg)

不要在入口扫描全pattern来配括号。对`abc`，`z(`未到达后缀而nil；`a(`走到模式末端，
准备成功返回时才因Open报unfinished。`(z`每个起点都匹配失败，不会仅因Open就报错。
只含`)`且无其它magic时走自动plain；要观察无Open却到达右括号的错误，用`^a)`。
这延续了上一包的“到达才验证”，而不是另设一套eager capture parser。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_capture_finish}}
```

当前配置最多32个捕获槽，普通、嵌套、位置和已经关闭的捕获都计入编号；不是只数
当前Open深度。第33次实际打开时返回明确Limit，未到达的第33个括号不提前报错。
unfinished/无Open关闭是Invalid→VM Type；32槽上限是Limit→VM Limit。官方合同只
冻结稳定的pcall错误标志，本地测试另外核分类、原因、参数号与发生时机。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract capture_errors_keep_success_path_timing_and_distinct_categories -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_nested_empty_and_position_captures_keep_distinct_kinds -- --exact
```

## 问题十一：只退回两个游标，为什么仍可能返回错误的捕获？

### 失败分支会修改旧槽，也会创建新槽

用`aaab`匹配`(a*)(a)b`，按0-based索引手推：

1. 打开#1得到Open(0)，a*贪尽到3；在尝试后缀`)(a)b`前保存检查点。
2. `)`关闭#1为Closed(0..3)，`(`新建#2为Open(3)，下一个a却遇到b，失败。
3. 重试a*只取两个a之前，必须把#1恢复Open(0)，并删掉#2。
4. 重跑后缀得到Closed(0..2)、Closed(2..3)，整体成功，返回`1,4,'aa','a'`。

只truncate到1槽，#1仍错误地关闭；全部clear又丢掉了回退点之前已有的起点。
`(a*)ab`仍保留作小例，但它不能单独证明旧槽与新增槽都正确恢复。

```diff
-Retry { minimum, next, pattern }
+Retry { minimum, next, pattern, captures: CaptureCheckpoint }
-captures.truncate(saved_length);
+restore_captures(&mut captures, &frame.captures);
```

这个diff把错误设计与当前机制并列，不冒称旧提交存在truncate代码。真实checkpoint
拥有完整的状态Vec；`states()`借出只读slice。复制的是枚举和offset/range，不是subject
数据；最多32槽的有界快照先让语义清楚，后续优化必须继续满足同一恢复合同。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_capture_checkpoint}}
```

![检查点恢复与持续计费](../images/v100-capture-checkpoints.svg)

### 捕获状态回退，已经付出的工作不退还

检查点不包含Budget。真正matcher保存快照前按saved槽数计费，重试恢复前按live与
saved槽数之和计费；逆向找Open、完成结果等也计工作。公共checkpoint/restore辅助
API不接受Budget，计费由matcher调用点负责，不能拿裸helper宣称有统一配额。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/string_find.rs:find_retry_state}}
```

Budget由同一次find独立拥有，在所有retry和候选起点之间共用。恢复捕获后，剩余
steps继续减少；若把工作计数一起恢复，失败分支便可以反复获得免费重试。新的
`match_at`拥有新的capture Vec，但借用的仍是同一个Budget；下一次find才另起预算。
retry深度限制的是同时保留的frame数，pop会减少当前深度；它不是累计消耗的steps。

对`aaaaab`和`(a*)aab`，真实API在30steps报Limit，在100steps成功捕获aaa；低预算
不是nil，也不是半成品捕获。该计数是候选实现的资源合同，不与官方正常pcall结果
冒充等价；固定work-counter fault的8→5→8只演示退款问题，不是实际算法的步骤数。

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract rollback_restores_closed_values_positions_and_new_slot_count -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_capture_and_work_limits_return_errors_without_destroying_the_session -- --exact
```

### 验收练习：完整恢复借来的检查点

只改`exercises/string_find_captures/starter.rs::restore_slots`。输入是工作Vec的
`&mut`短借用和真实checkpoint借出的`&[CaptureState]`。恢复旧槽的每个状态、删除
多出的槽，保持saved不变且可再次借用；不要改合同、solution、faults、probes或ignore。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_find_captures/starter.rs:capture_restore_starter}}
```

参考实现把checkpoint内容复制回工作Vec，不持有指向checkpoint内部的长期引用：

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_find_captures/solution.rs:capture_restore_solution}}
```

合同故意既改Open/Closed/Position的旧值，也追加新槽，并连续恢复两次；只断言长度
会漏掉损坏。hook是恢复投影，没有Budget借用；真实matcher的计费与错误后继续另验。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_find_captures/contracts.rs:capture_restore_contract}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference checkpoint_restore_replaces_old_slots_and_removes_new_slots -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_exercises checkpoint_restore_replaces_old_slots_and_removes_new_slots -- --ignored --exact
```

第一条验参考，第二条选learner，初始必须失败，补完后才通过；每条恰选一项测试。
Release在`--`前加`--release`。本包还有8项参考覆盖真实回退、空/位置、不同起点、
owned bytes、上限与helper一致性；详见[捕获练习包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/exercises/string_find_captures/README.md)。

## 问题十二：matcher借用输入，返回的捕获字节最后由谁拥有？

### 从一次调用的索引，走到调用之后仍有效的值

`FindMatch`拥有span和捕获范围元数据，没有拥有subject bytes；调用者解读范围仍要
保留原subject。matcher借用VM adapter本次的参数副本，不在内部先分配每个尝试的
捕获字符串。成功后，adapter按编号一次把Bytes(range)复制为现有ByteString，把
Position(offset)转为offset+1的Number，追加在整体两个坐标之后。

```diff
-return values_that_borrow_local_subject;
+// Keep offsets while matching; materialize only the successful captures.
+Value::String(ByteString::new(haystack[range].to_vec()))
+Value::Number((offset + 1) as f64)
```

前一行是会受局部输入生命周期限制的拟议接口，不是历史实现。当前ByteString拥有
Box<[u8]>；这是Rust所有权问题，没有引入新的GC根，也不把范围元数据说成字符串。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/crates/lua_vm/src/lib.rs:string_find_owned_captures}}
```

![捕获返回值所有权](../images/v100-capture-ownership.svg)

每个候选起点的新Vec防止失败尝试泄漏：`find('axab','(a)b')`应为`3,4,'a'`，不能
夹带前一次已关闭或未闭合的槽。失败不交付成功半成品，不回滚此前Lua的globals/I/O。
保存含NUL/0xff的返回Value，再做GC与下一次find，最后释放Session，才检查原bytes：

```rust,ignore
{{#include ../../code/v100-lua51-runtime/exercises/string_find_captures/probes.rs:capture_real_owned_bytes}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_captures_reference actual_vm_returns_owned_binary_capture_values -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract owned_indices_vm_strings_and_legacy_range_projection_keep_their_boundaries -- --exact
```

### 数字坐标合同不能自动验字符串捕获

原75个case与driver逐byte不改，继续验原样坐标/元数输出。新增54个手写固定场景
使用独立capture fixture/driver：数值仍为`n:`，字符串为`s:长度:小写hex`，空字符串
为`s:0:`，nil仍是单独一个值；保留完整arity。固定binary case用`'A'..char(0,255)..'B'`
匹配`(%z.)`，stdout为`ok|3|n:2|n:3|s:2:00ff|after`，不让Windows stdout文本换行或UTF-8解码
掩盖payload。driver不读取TSV，也不用find/format生成expected。

```rust,ignore
{{#include ../../code/v100-lua51-runtime/tests/string_find_capture_contract.rs:capture_binary_contract}}
```

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract real_binary_matches_fifty_four_fixed_capture_byte_cases -- --exact
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --test string_find_capture_contract capture_corpus_keeps_fifty_four_arities_lengths_and_binary_values -- --exact
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --suite all --check
```

--check只做schema/driver检查，两端明确NOT_RUN。实际对照从仓库根构建binary后运行：

```powershell
cargo build --manifest-path book/code/v100-lua51-runtime/Cargo.toml --bin lua_app
py -3 book/code/v100-lua51-runtime/tools/check_string_find_contract.py --suite all --candidate-bin book/code/v100-lua51-runtime/target/x86_64-pc-windows-msvc/debug/lua_app.exe --official-bin target/oracles/lua-5.1.5/build/Release/lua51.exe --official-archive target/oracles/lua-5.1.5/lua-5.1.5.tar.gz --output target/string-find-captures-review
```

旧75与新54各自对固定expected验双方完整双流和退出码；Python实际执行和Rust
集成执行单列计数，复跑不计新增覆盖。独占case/run目录归档stdin、原始双流、metadata
和输入/binary/source SHA，超时后kill仍wait；源tar与真实官方binary身份分开记录。
错误case的pcall false不替代本地分类检查；traceback文字和候选资源预算不冒称官方
逐字相同。参照[Lua5.1捕获规则](https://www.lua.org/manual/5.1/manual.html#5.4.1)及
[5.1.5源码](https://www.lua.org/source/5.1/lstrlib.c.html)的start/end_capture、push_captures；
永久CLI和身份说明见[find官方合同](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v100-lua51-runtime/tools/string_find_contract/README.md)。

新增capture能力行指向本地合同，但byte-string、format、find基础和capture四行都
保持 **LUA51-D005 partial**。backreference、balanced/frontier、optional/non-greedy、
gmatch/gsub、其它format/dump和locale/资源边界继续登记；不把129个场景说成完整pattern。

## 38 Opcode：语义覆盖不等于二进制兼容

Lua 5.1 `lopcodes.h` 定义 38 个 Opcode。教学 VM 从 v0.5 起使用自己的 32 位寄存器格式，v0.18 已演进为 39 个更显式的 Opcode：

| Lua 5.1 Opcode | 教学 ISA | 原因 |
|---|---|---|
| `LOADBOOL` / `LOADNIL` | `LoadK` | boolean 与 nil 都进入普通常量表 |
| `SELF` | `GetTable + Move` | 方法查找和 self 布置显式展开 |
| `TESTSET` | 条件跳转 + `Move` | 短路复制由 codegen 明确表达 |
| `TFORLOOP` | `Call + JumpIfNil + Move` | iterator 协议拆成可观察步骤 |
| `SETLIST` | `SetList` / `SetListOpen` | 固定宽度与 MULTRET 分开 |

`lua_bytecode::compatibility` 按官方顺序定义 `Lua51Opcode`，并为全部 38 项给出非空映射、direct/composite 标记与说明。测试验证 ordinal 0–37 没有重复或缺项。运行：

```powershell
cargo run --manifest-path `
  book/code/v100-lua51-runtime/crates/lua_bytecode/Cargo.toml -- `
  --format opcode-audit
```

输出 schema 为 `lua-rust-lua51-opcode-audit/v1`，同时固定 `binary_chunk_compatible=false`。源码语义覆盖属于 v1.0，Lua 5.1 binary chunk 编解码与 verifier 属于 v1.1。

## 冻结 bytecode 与 trace 工具协议

v0.18 的 JSON 有稳定形状但没有版本号。本版给 Proto JSON 增加 `lua-rust-bytecode/v1`，给 VM trace 增加 `lua-rust-vm-trace/v1` 顶层 envelope：

```json
{"schema":"lua-rust-vm-trace/v1","events":[...]}
```

byte string 仍以 hex 输出，NaN/Infinity 仍不能冒充 JSON number。进程测试锁定 schema、错误退出码和 38-op audit 的行数。添加兼容字段可保持 v1；改变既有字段语义则必须升级 schema。

## 双 oracle 不是“双份 expected string”

官方 Lua 5.1.5 是语言参照，固定 C++ commit 是工程迁移参照。身份固定为：

- Lua 5.1.5 source SHA-256：`2640fc56...b0738695333`；
- official tests SHA-256：`49e4ca65...67f18ad`；
- C++ oracle commit：`87c15e69...635711`；
- exact-commit codeload archive SHA-256：`66c9506a...1bbfb8`。

`tools/run_differential.ps1` 对每个 fixture 启动三个真实进程，只做已声明的 CRLF→LF 归一化，然后比较 stdout、stderr 与 exit code。每个进程有十秒 timeout，超时只终止该 child process tree。

本次实际基线为 5/5 fixture 同时匹配两个 oracle：

| Fixture | 覆盖 | 结果 |
|---|---|---|
| `language-core` | 循环、多返回、短路、拼接 | 双 lane 通过 |
| `byte-strings` | NUL、0xff、byte/sub/length | 双 lane 通过 |
| `metatables` | `__index` 与 `__call` | 双 lane 通过 |
| `coroutines` | yield/resume 多值与状态 | 双 lane 通过 |
| `core-stdlib` | math/string/table 代表接口 | 双 lane 通过 |

baseline 只证明这五个可观察合同，不等于官方全量测试套件通过。依赖 C API、binary chunk、native module 或尚登记标准库差异的部分不能被静默过滤；能力矩阵必须保持 `partial`/`excluded`，直到对应差异真正关闭。

## Release gate

普通快照验收不依赖外部下载：

```powershell
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --workspace
```

完整本地发布门执行 Debug/Release 测试、严格 Clippy、示例 stdout、38-op audit；提供 oracle 路径后再执行双差分：

```powershell
book/code/v100-lua51-runtime/tools/release_gate.ps1 -RunOracles `
  -OfficialLua path/to/lua51.exe `
  -CppLua path/to/lua_app.exe `
  -CppRoot path/to/pinned-source
```

不传 `-RunOracles` 时脚本明确打印 skipped，而不是把缺少 oracle 当作通过证据。

## Rust 知识映射与设计选择

- `#[repr(u8)]` 和固定数组把官方 opcode ordinal 变成可编译的不变量；
- `&'static [Opcode]` 表达一个官方语义由多个教学指令组合；
- `BTreeMap/BTreeSet` 让 registry 的重复、引用和顺序检查确定；
- versioned envelope 把 Rust struct 演进与 JSON consumer 解耦；
- 子进程异步读取 stdout/stderr，避免管道缓冲导致 differential deadlock；
- source/archive hash 与 commit 一起固定，避免 oracle 随网络分支漂移。
- `Repl<'a>` 借用唯一 Session，`Option::take` 将 EOF 诊断所有权转移一次；
- 独立快照复制固定 byte fixture 与测试，仓库身份 gate 阻止副本分叉。
- `FormatSpec` 与 `Option<u8>` 区分未指定和显式0；共享 byte slice 与局部 cursor
  分离解析/提交，Result 将格式、缺参和类型错误带回 VM；
- 符号、数值精度与宽度补齐分开，按规则处理优先级，不依赖通用 Display。
- find用 `Result<Option<Range<usize>>, FindError>` 分离错误、未命中与空成功；
  Atom借用pattern、Vec保存回退状态，让错误时机与候选顺序可以直接测试。
- capture用enum区分工作态与成功态；拥有的检查点Vec与只读slice分开，完整恢复
  旧值/槽数。Budget独立借用且持续计费，ByteString在成功边界拥有拷贝出的原bytes。

替代方案是把教学 ISA 强行改成官方 38 条并立即读取官方 chunk。这会把语言收口与二进制格式、端序、数字宽度、安全 verifier 混成一次破坏性迁移，因此留给 v1.1。另一个方案是只保存 golden output；真实双进程差分能在平台和错误合同改变时暴露差异，证据更强。

## 从 v0.18 迁移了什么

- 新增：Lua string/table call sugar 与 parenthesized prefix statement；
- 修复：help 不再在 option parser 内调用 `process::exit`，而是正常 action；
- 冻结：bytecode JSON 与 VM trace v1 schema；
- 新增：官方 38 Opcode enum、完整语义映射和机器可读 audit；
- 新增：能力矩阵、偏差注册表、oracle manifest 与 verified baseline；
- 新增：5 个双 oracle fixture、provenance-aware runner 和 release gate；
- 保留：v0.18 的语言、VM、GC、Coroutine、标准库、模块、CLI 与工具测试；
- 显式移植：结构化 CompileErrorKind、同一 Session 的 Repl 与真实 LF/EOF adapter；
- 共用合同：26个 raw byte 场景，本地自含副本与仓库身份/进程 gate；
- 收口子集：既有 string.format 的规格/参数边界与57场景官方字节合同，D005仍partial；
- 收口子集：find的plain/init、字节atom和`*`/`+`回退与75场景官方合同，D005仍partial；
- 收口子集：find普通/嵌套/空字符串/位置捕获、检查点恢复与owned返回，另54场景，D005仍partial；
- 未伪装：标准库剩余差异和 v1.1+ 能力继续显式登记。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v100-lua51-runtime/Cargo.toml -- `
  book/code/v100-lua51-runtime/examples/demo.lua
cargo test --manifest-path book/code/v100-lua51-runtime/Cargo.toml --workspace
```

示例输出：

```text
100
```

Release gate 还要求相同测试在 `--release` 下通过、Clippy 零 warning、示例 stdout 精确匹配，并检查 38-op audit 没有宣称 binary compatibility。

## 本版本暂不支持

- Lua 5.1 binary chunk、`string.dump` 与官方 opcode 二进制编码（v1.1）；
- Lua C API/ABI、headers、lauxlib 和外部 C consumer（v1.2）；
- `package.loadlib` 与动态 native module（v1.3）；
- string pattern 全集、完整 `string.format`、函数 environment API；
- io/os/package/debug 的全部平台与边缘 API；
- C runtime `rand` 序列、完整 locale/timezone/DST 合同；
- Windows argv/path 的无损宽字符 adapter；
- readline、signal-aware REPL、沙箱、配额和跨平台 SDK。

后三组 v1.0 范围内差异分别由 `LUA51-D004`–`LUA51-D011` 登记；它们不是未声明通过项。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题</strong>
  <p>选择一个 composite opcode，从源码行为、教学 bytecode 与 VM trace 三层描述证据，并
  解释为什么仍不能读取官方 binary chunk。</p>
</div>

<div class="exercise implementation">
  <strong>实现题</strong>
  <p>给 capability evidence 增加内容 hash，检测文件存在但证据漂移；同时为错误 fixture
  设计不依赖绝对路径的分类 schema。</p>
</div>

<div class="exercise debugging">
  <strong>调试题</strong>
  <p>故意删除一条 deviation 反向引用或给 complete 项绑定 deviation，运行 release test，
  沿失败信息验证闭世界检查能准确指出非法关系。</p>
</div>

<div class="exercise understanding">
  <strong>理解题：迁移后谁还拥有原诊断？</strong>
  <p>对照状态图，逐步写出 source、pending 与 Session 的 owner。成功条件是能说明
  NeedMoreInput 不执行前缀，EOF take 后再 finish 不报错，普通错误不会新建 VM，
  以及真实 LF 与末片段无 LF 为什么走不同分类。</p>
</div>

<div class="exercise implementation">
  <strong>实现题：补齐迁移包的一个 EOF hook</strong>
  <p>只补 starter::finish，先运行 --ignored 精确验收，再运行真实 controller 与两版
  byte 合同。要求原诊断只返回一次、清空输入，并保留 v1.0 调用语法糖与 Help action。
  固定参考、示例或默认 ignored 结果均不算完成 learner。</p>
</div>

<div class="exercise debugging">
  <strong>调试题：相同错误输出为什么骗过相互比较？</strong>
  <p>运行三个 faults，指出每次状态丢失或复制的位置；再说明固定 oracle 如何拒绝
  两边都静默 EOF 的实现。成功条件是用完整双流 bytes、退出码与 source/Option 状态
  给出证据，并区分副本身份检查、实际行为检查和官方语言 oracle 的覆盖范围。</p>
</div>

<div class="exercise understanding">
  <strong>理解题：两个零为什么不是同一个含义？</strong>
  <p>分别手推 %f、%.f、%05.3d 的 spec、默认精度与输出。成功条件是说明 None、
  Some(0)、零宽度 flag 的区别，并解释整数显式 precision 为什么改变补零规则。</p>
</div>

<div class="exercise implementation">
  <strong>实现题：补齐格式化包的参数提交 hook</strong>
  <p>只补 consume，要求成功后前移一次、缺参或类型错误保持 cursor；运行 --ignored
  exact，再验真实 parser、VM 和固定57场景。Args 投影通过不等于真实 formatter
  的数字范围、NUL 和字节渲染均已正确。</p>
</div>

<div class="exercise debugging">
  <strong>调试题：expected 自己写对了吗？</strong>
  <p>复现旧 %f 预期、单byte规格与 table stringify 三个故障，定位每次必要改动；
  再解释为什么直接捕获 Windows stdout 中的 q 换行会混入文本模式转换。成功条件
  是从固定oracle、Lua层hex、本地错误分类各给证据，且D005仍准确标partial。</p>
</div>

<div class="exercise understanding">
  <strong>理解题：没有byte，为什么仍然成功？</strong>
  <p>把None、Some(0..0)、Some(3..3)、Err分别映射到Lua结果或错误；手推plain=0、
  init=99与NUL分流。成功条件是能区分返回一个nil与返回两个空区间坐标。</p>
</div>

<div class="exercise implementation">
  <strong>实现题：只提出下一次重试的位置</strong>
  <p>只补next_position，候选降序、每项一次、Some(0)有效，plus不低于一次；运行
  --ignored exact，再验真实atom、matcher与官方75场景。hook不自己判断后缀。</p>
</div>

<div class="exercise debugging">
  <strong>调试题：未闭集合为什么有时没有报错？</strong>
  <p>对照z[与a[、贪尽不退与逐步退、Limit与None三组反例。成功条件是标出当前
  subject/pattern位置和Retry，解释哪一步到达错误，并说明eager parser为何改变行为。</p>
</div>

<div class="exercise">
  <strong>理解题：整体范围相同，第三个值为什么不同？</strong>
  <p>预测find('b','(a*)')与find('b','()')，列出整体Range、capture工作态、成功态和
  Lua返回元数/类型。再手推((a)b)的编号与关闭顺序，解释为什么未闭合不能用0终点表示。</p>
</div>
<div class="exercise">
  <strong>实现题：恢复完整的capture检查点</strong>
  <p>只补string_find_captures/starter.rs::restore_slots。恢复Open/Closed/Position旧值，
  删除新增槽，让同一saved可再次使用；逐项写下哪一个owner负责live、snapshot和Budget。
  显式运行learner exact，初始失败，修改后通过，不能改ignore或参考答案。</p>
</div>
<div class="exercise">
  <strong>调试题：位置退回了，capture和预算分别该怎么办？</strong>
  <p>对aaab与(a*)(a)b，标出checkpoint、live旧槽/新增槽和下一候选。分别用truncate、
  clear、恢复工作计数三个fault解释错在哪里。再保留二进制capture返回Value，GC并释放
  Session后检查00ff，说明索引元数据与owned ByteString各自能保证什么。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

读者应能解释：为什么双 oracle 比单个 expected output 强；为什么“38 项语义映射”不等于“binary chunk 兼容”；为什么偏差注册表必须双向闭合；以及为何外部 oracle 缺失必须显示为 skipped 而不能假绿。

还应能在本地快照完成分类→controller→adapter 的小步迁移，保留本版 parser/main
增量；手推 NeedMoreInput、普通错误、idle/pending EOF 与 I/O Err 的状态；完成 EOF
TODO；独立构建 v1.0 并让两版逐字节满足同一固定合同，而非只互相相等。

格式化还应能从一个失败输入推导 FormatSpec、默认精度与符号/补齐的顺序，区分
格式游标与参数游标；完成 consume TODO；解释短串 NUL、长串 fast path 与 q 回读
的不同合同，并用官方字节证据支撑子集进展，保留D005的剩余边界。

find还应能手推init/空区间和C-prefix分流，解释借用Set的生命周期与按到达才解析；
完成Retry TODO，画出a*ab的失败/回退轨迹，说明*与+的下界；用官方元数/坐标合同
和候选专用Unsupported/Limit检查各自举证，不外推完整pattern或无限资源。

捕获还应能解释Open/Closed/Position与成功态的区别，手推同时改旧槽/添新槽的回退；
完成restore_slots TODO，证明只改Vec长度不足。说明预算为何不在checkpoint内、快照/
恢复如何计费；解释VM在哪一步拥有捕获bytes，并让原75与新增54场景各自逐byte通过。

## 与成熟实现和专题章节的对应

- 成熟实现的 differential runner、M0/M1 gate、official source manifest 和 living deviation log 是本版最小发布治理的扩展目标；
- [Bytecode](../ch03-compiler/bytecode.md) 对照教学 ISA 编码；
- [寄存器虚拟机](../ch04-vm/register-vm.md) 对照 composite opcode；
- [用测试守住兼容性](../ch05-engineering/testing.md) 深入 fixture、oracle 与进程级证据；
- 根目录 `book_plan.md` 继续定义 v1.1–v1.4 路线。
- 下一步可由backreference的最小fixture继续推进捕获读取/未闭合引用的错误边界，
  或选择另一个登记构造；%b/%f、?/-、gmatch/gsub与REPL的TTY/signal/增量parser分别
  立项。REPL26、format57、find基础75和capture54各保留自己的覆盖与历史证据归属。
