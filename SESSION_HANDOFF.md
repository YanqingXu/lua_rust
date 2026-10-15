# 未完成目标交接

更新：2026-10-15，Asia/Shanghai。此文件是跨会话接续入口；详细整改状态见
[审计整改](docs/audits/2026-09-12-remediation.md)，长期计划见 [plan.md](plan.md)。

## 2026-10-15：v1.0 string.find 有界兼容问题链

用户要求收口plain分流、init/空匹配、字节类/转义及`*`/`+`贪婪回退，补齐反例、
diff、状态图、练习和官方Lua5.1.5字节对照。本批实现、教学与验证已完成，
**没有恢复暂停的原审计目标**；`LUA51-D005`仍为partial。用户随后明确要求将本批
实现、官方字节合同、章节、状态图、练习包、能力证据和交接记录一并提交推送。

- 起点HEAD与本地origin/main均为已推送`5347c60ee5176054a792265b2c098298e8a7dd79`，
  开始时工作区干净。仅改v100 VM adapter、新string_find模块、两个Cargo test target、
  本版教学/练习/字节合同及能力证据。生产根crates、v18、v100 compiler/core/bytecode/
  stdlib本体、REPL、format及其既有合同、Cargo.lock无diff；无新第三方依赖。
  同期另一个调试入口工作包修改`.vscode/launch.json`、`book/code/README.md`和下方
  VS Code交接段，均保留；提交时仅暂存本批find段，三项同期修改不纳入find提交。
  提交前fetch确认HEAD与origin/main无分叉；本批提交号以
  `git log -1 -- book/src/versions/v100-lua51-runtime.md`为准，新提交CI按其SHA查询。
- [主章](book/src/versions/v100-lua51-runtime.md)新增问题七至九：总做字面查找/空匹配
  混同nil → Result/Option/半开Range；整条pattern预检 → 到达atom才解析；贪尽不退
  → 显式Retry栈。每问有预测、固定faults、真实API/VM观察、最小diff、≤60行源码
  include与exact验收；新增11个include、三幅SVG、三类练习、迁移和检查点。
- find API返回`Result<Option<Range<usize>>, FindError>`，`Some(3..3)`映射Lua(4,3)，
  不能当未命中。init归一到0..=len，含负数与极端i64；plain按truthiness，0为真。
  默认模式只在pattern首NUL前扫描magic：无magic走完整byte字面路径，有magic才解释
  C-prefix；subject始终保留全bytes。NUL后的magic与NUL前的magic分流不同，0xff不经
  UTF-8或char转换。`^`锚在归一化起点，末尾`$`检查subject末端，未宣称完整pattern。
- Atom以borrowed slice表达集合，ASCII/C-locale十类及大写补集、范围/取反/转义由
  实际parser处理；`%w`不含下划线，未知`%g/%G`是字面g/G。`[]`缺闭合会报错，
  `[]]`可匹配`]`。未到达的`z[`/`z%`返回nil，到达`a[`/`a%`才Invalid；空subject
  仍会检查首atom。Invalid/Unsupported映射VM Type，Limit映射VM Limit，分开验收。
- `*`/`+`先尽量消费，再按候选位置递减重试后缀，`+`保留至少一byte，允许候选0；
  Vec保存minimum/next/pattern，无宿主递归。默认matcher预算100000 steps与128层
  backtrack，显式guard有专测；plain路径未计入此预算，不冒称官方复杂度策略或时间
  上限。captures/backrefs/%b/%f/可选?/非贪婪-仍按到达位置Unsupported，不能把这些
  字节在plain或集合中的字面用法一并禁掉。
- [练习包](book/code/v100-lua51-runtime/exercises/string_find/README.md)含9参考、
  1个next_position TODO、faults5/find8/all13条unique marker/profile。Retry教学投影
  只实现候选位置回退，借用真实parser/VM另验，没有复制matcher。D/R各完成初始101
  →临时套解0→恢复原bytes并刷新mtime→再101；交付保留TODO。练习36条命令和10个
  源文件哈希稳定。固定ignore_plain fault强制走新matcher，是设计反例；旧binary
  实际策略是始终literal，二者在文中区分。
- [官方合同](book/code/v100-lua51-runtime/tools/string_find_contract/README.md)有75个
  手写固定expected，独立Lua driver用返回arity和数值编码，不调用find/format生成
  expected。完整stdout为ok|arity|values|after或error|after，stderr空、exit0，原始
  双流不trim/normalize。Rust另验参数号、kind、错误到达时机及候选专属unsupported/
  limits，不能仅凭pcall false误绿。未知id/driver编译错误在Lua pcall之外，宿主仍
  捕获双流并拒绝。--check只验schema/driver，实际两端明确NOT_RUN。
- 首次官方核对纠正一条手写假设：`[]`不是空集合nil。原74/75失败raw证据保留，按
  官方classend修正为error，并加入`[]]`正例，最终75/75；未把第一次写成全通过。
  官方源码tar SHA2640fc56…b0738695333、真实x64 PE8664 binary SHA47fd81e4…693978d，
  -v为5.1.5；分别记录源码与binary身份，不冒充可复现构建证明。
- Python独立D/R双方共300个raw case逐byte通过且全部回收；Rust两profile另150次
  实际进程case，workspace自然重复执行不计新增覆盖。冻结旧5347 binary的SHA为
  132f5569…5109e2，与上一format批Debug产物一致；12组旧/官方baseline共24次真实
  观察。旧binary跑同75场景被拒49项、官方75/75、CLI exit1；该150次负控与300次
  正向分开。缺oracle参数exit2、错tar SHAexit1；communicate并发drain，超时kill
  后仍wait，仅承诺本批无后代的有界子进程，不外推任意进程树治理。
- capabilities新增find子集partial行→tests/string_find_contract.rs→D005；byte-string、
  format、find三行均partial。D005继续登记captures/backrefs/balanced/frontier、?/−、
  gmatch/gsub、其它format/dump、locale/浮点/整数边缘与matcher资源限制；75场景通过
  不代表完整Lua string library。2026-08-21历史双oracle baseline保持原版本归属。
- Windows Rust/cargo1.96.1：本地release_gate一次，fmt、all-targets Clippy -Dwarnings、
  D/R workspace各204通过/0失败/3ignored（原180+VM9+参考9+合同6）。两profile bins、
  example faults5/find8/all13逐行验收、demo100、stdlib1550通过；旧REPL26/format57
  由workspace回归，旧语言双oracle明确skipped，新find官方字节合同单列。
- 全新独立snapshot copy含82代码输入、6本地packages，全部path依赖闭合、第三方0；
  不共享target，offline/locked构建workspace bins、本地6合同/75case与demo100通过。
  继承祖先Rust linker配置，无v18/根tools源码依赖。全量84永久输入前后SHA稳定；
  runtime2、练习10、合同79输入分别冻结。18条全量命令均通过，冻结验证器SHA为
  3d519836ae5235ef1b29c96dd4f0ab0adab62b5d669ad342a25f24d0134f274d。
- 主章与3份README提取16条去重exact，D/R共32次：30正向通过、2learner按预期
  exit101，每条恰选1test；82个源码/fixture/tools/文档输入SHA前后稳定。只读审阅
  确认表中四组历史输入与归档raw一致，11个include均≤60行；证据为
  `doc-exact/20261015T122506421737Z/summary.json`。
- 本机ignored证据根`target/teaching-v100-find-2026-10-15/`：baseline见
  `baseline-20261015T121034943988Z/summary.json`；runtime见
  `runtime/20261015T121434829819Z/summary.json`；练习套解见
  `exercises/20261015T121423019187Z/summary.json`；官方合同/负控见
  `contracts/20261015T121742437303Z/summary.json`；全量见
  `full-validation/20261015T121829749882Z/summary.json`。文档精确命令、最终范围、
  HTML/include、链接及图审阅汇总见本根`final-verification.json`和`visual-review.json`。
- 20版本教学检查、mdBook及38页1403站内链接/0broken通过。三SVG用bundled sharp/
  libvips渲染PNG并直接查看文字、箭头、边界和页脚；Fontconfig缓存告警不影响本次
  渲染。另验生成HTML的include/图片引用，不把PNG审阅冒称浏览器页面视觉验收。
- 基线5347的[mdBook 38728934980](https://github.com/YanqingXu/lua_rust/actions/runs/38728934980)
  成功；[Rust 38728935001](https://github.com/YanqingXu/lua_rust/actions/runs/38728935001)
  completed/failure，三个Miri/ASan job成功。Windows M1质量门仍因生产fetch_update
  弃用的严格Clippy失败，official source三方10/10；baseline-book/rust.json与真实
  baseline-rust-job.log保留。本批未修改生产CI，不能宣称远端绿色。

下一工作包可从固定capture反例推进捕获槽位、回退后的capture状态恢复与返回值所有权，
仍先固定官方bytes再实现；%b/%f、?/−、gmatch/gsub及剩余format边界另分工作包。
原审计仍暂停在os.date/os.time本地时间、UTC、DST。本交接不表示恢复；本批按用户
明确指令提交推送，保留同期VS Code工作包，最终提交SHA的CI状态需另行查询。

## 2026-10-15：book/code 的 VS Code Rust 调试入口

本包响应用户的教学快照调试配置问题，**没有恢复暂停的原审计目标**，未提交或推送。
开始核对 HEAD 与本地 origin/main 均为 `5347c60ee5176054a792265b2c098298e8a7dd79`；
该 SHA 的 [mdBook 38728934980](https://github.com/YanqingXu/lua_rust/actions/runs/38728934980)
成功，[Rust CI 38728935001](https://github.com/YanqingXu/lua_rust/actions/runs/38728935001)
失败于 Windows 的 M1 foundation and quality gate，三个 Miri/ASan job 成功；本包未整改 CI。

- 仅新增 `.vscode/launch.json` 的四类入口、`book/code/README.md` 调试说明及本检查点。
  原三个根工程入口保留；现有 v100 string_find 实现/合同/教学未提交工作未纳入本包。
- v00–v04 使用 lua-step，v05–v100 使用 lua_app；选择器覆盖全部20版本。通过独立
  manifest 和 bin/example/lib 目标由 Cargo 返回产物，避免启动根工程同名 exe。
  cwd 保留仓库根目录，以适配 v17 demo 的 package.path；Rust example 默认 all，
  更改参数需编辑 args 数组。单元测试默认 v100 lua_vm 的 formatter parser 测试。
- 本地已有 rust-analyzer 0.3.3081、CodeLLDB 1.12.3。四类默认入口均实际完成
  offline/locked Debug 构建、唯一 artifact/PDB 检查、运行及 bundled LLDB 22.1.8
  源码断点命中，继续后均 exit0；单元测试恰1项，example 恰12条 ok。
  校验期间已有 v100 未提交文件 SHA 未变；这是当前工作区的定向调试验证。
- 本机 ignored 证据：`target/book-code-debug-2026-10-15/20261015T121927309368Z/summary.json`
  及各入口 build/run/lldb 原始双流。前两次校验因验证脚本对 PDB 文件命名的假设
  停止，修正验证器后上述完整运行通过；未把前两次记成配置失败或完整通过。
  未驱动 VS Code UI/输入对话框，不把 CLI 断点验证冒称编辑器 UI 实测。

下一入口：按 `book/code/README.md` 从仓库根目录打开 VS Code，设置所选快照 `.rs`
断点并按 F5。需要编辑器分析时，仅关联当前学习版本的 linkedProjects；无需恢复审计。

## 2026-10-15：v1.0 string.format 有界兼容问题链

用户要求先收口已有转换项的默认精度、flags/宽度/精度、参数类型与错误边界，补齐
反例、diff、状态图、练习与官方Lua5.1.5字节对照，更新能力证据并保持D005 partial。
本批实现、教学与验证已完成，**没有恢复暂停的原审计目标**。用户随后明确要求将
本批实现、官方字节合同、章节、状态图、练习包、能力证据和交接记录一并提交推送。

- 起点为已推送 `main@74fe6ca0efa597db4053039e8a13842b2a39d940`，开始时工作区
  干净、origin/main跟踪同SHA。仅修改v100 VM adapter、新string_format模块、旧
  stdlib测试的错误%f预期、Cargo/lock本地依赖与教学/证据文件；生产根crates、
  v18、v100 compiler/core/bytecode/stdlib本体、REPL实现及既有字节合同均无diff。
  lua_app为练习直接引用已有lua_stdlib path包，没有引入第三方、FFI或C varargs。
  提交前fetch确认HEAD与origin/main无分叉。本批提交号以
  `git log -1 -- book/src/versions/v100-lua51-runtime.md` 为准，新提交CI按其SHA查询。
  提交前暂存检查识别到TSV空末列必需的尾部tab；.gitattributes仅为该fixture指定
  LF及whitespace=-blank-at-eol，保留原数据SHA和其它路径的默认空白检查。
- [主章](book/src/versions/v100-lua51-runtime.md) 新增问题四至六：错误绿色测试
  → 默认precision6；单byte读取 → FormatSpec与局部cursor；宽泛stringify → 类型/
  缺参/错误优先级。每问含预测、可运行faults、最小diff、≤60行源码include与exact
  验收；两幅新SVG解释规格解析/补齐优先级与格式/参数双游标。章末/README补齐三类
  练习与检查点，保留REPL上一批及2026-08-21历史双oracle证据归属。
- 真实模块公开Flags、FormatSpec、parse_spec，start紧接%、next紧接conversion。
  flags为-+ #0，按字符数≤5（++允许，第6个才错）；width/precision各最多两位，
  None和Some(0)分开，空precision为0。已有s/d/i/f/q/%%保留；f默认6位，处理负零、
  符号后补零、左对齐/+优先级、整数precision抑制zero宽度、#保留小数点。
- format_arguments统一首参数缺参/类型与数字转字符串路径，s/q数字按局部%.14g
  近似与列明代表值验收，不改变全局Args。d/i共同signed i32范围，非有限与超范围
  返回错误；宿主CRT/locale/浮点舍入边缘与其它转换仍明确partial。该范围收口不
  宣称所有旧i64数值和官方不同整数配置等价；范围guard有unit验收与偏差登记。
- dispatcher普通byte/%%不取参数，其余先检查缺参再parse规格、类型/范围与render，
  成功后提交局部结果和双游标。Type错误保留真实Lua参数号；局部Vec不返回成功
  半成品，不回滚VM此前globals/I/O。短%s按C-NUL截断，无precision且len>=100时
  原bytes直通并绕width；显式precision禁fast path。q按5.1.5生成5c0a物理换行、
  NUL/引号/反斜杠转义，另经实际compiler/VM及loadstring回读原bytes。
- [练习包](book/code/v100-lua51-runtime/exercises/string_format/README.md)：1个
  consume TODO、9参考，fixed faults四条、真实format八条、all十二条unique marker。
  spec借真实parser，values借完整参数slice且cursor从1开始，复用Args验证后才前移；
  Value clone不深拷贝heap。它是类型/缺参与游标提交投影，数值范围、字符串转数字
  边缘与render另验，不伪造第二parser或VM回滚。D/R TODO初始101→临时solution0→
  原bytes恢复并刷新mtime→再次101，交付保留TODO；旧REPL题仍归上一批。
- [官方合同](book/code/v100-lua51-runtime/tools/string_format_contract/README.md)：
  57个手写固定expected先由真实官方核对；独立driver按id执行，不读取expected，
  编码器用byte/sub/整数运算，不调用format。为区分Windows C stdout文本换行与
  内部q bytes，Lua层先编码ASCIIhex，捕获后不trim/normalize双流。完整stdout为
  ok|hex|after或error|after、stderr空、exit0；Rust另3组检查全部16类错误的kind/
  参数号/分类/缺参优先级，不能仅凭pcall false误绿。未知id/assert在pcall外。
- 永久Python gate --check只检查schema/driver并明确两端NOT_RUN；实际运行强制
  candidate+official+固定源码归档，独占run/case保存实际/预期原始双流、空stdin、
  metadata与SHA。communicate并发drain，超时kill后仍wait；有界无后代，不外推
  任意进程树/fsync。官方源码tar SHA2640fc56…b0738695333，实际官方x64 PE8664、
  binary SHA47fd81e4…693978d，实际-v为5.1.5。分别记录源码与binary身份，不把
  版本字符串说成可复现构建证明，也不冒充本轮执行了固定C++双oracle。
- capabilities新增format子集partial行→tests/string_format_contract.rs→D005，
  原byte-string行仍partial。D005描述保留pattern、其它format转换、dump、宿主
  printf/locale/float边缘、非有限值与超signed i32范围；没有把57条升级为complete。
- 本地Windows Rust/cargo **1.96.1**：release_gate一次执行，fmt、all-targets
  Clippy -Dwarnings、D/R workspace各 **180通过/0失败/2ignored**（原158+VM8+
  参考9+合同5）。两profile bins、faults4/format8/all12逐行验收，demo100、stdlib1550
  通过。既有REPL26场景由workspace两profile回归，源码/fixture均未改；legacy语言
  双oracle明确skipped，新format官方合同单独验证，不借历史baseline冒充通过。
- Rust进程合同两profile各57case；独立Python双方D/R共 **228个raw case**逐byte
  通过且全回收。先冻结上一批实际旧binary9efe6dc…，SHA与上批记录相符并捕获
  8组旧/官方raw baseline；旧binary跑同57场景被拒33项、官方仍57/57，CLI exit1，
  负控114次与新228次分开。缺official参数exit2、错tar SHAexit1亦按预期失败。
- 全新独立snapshot copy有68代码输入、6本地packages、path依赖全部闭合、第三方0；
  不共享target，offline/locked构建workspacebins、本地5合同/57cases与demo100通过。
  继承祖先Rust linker配置，无v18/根tools源码依赖。全量70输入SHA前后无漂移，
  包括Cargo.lock、能力/偏差、driver/fixture/Python gate，练习/合同各层冻结另留。
  主章与3README去重16条exact，D/R共32命令：30正向通过、2learner预期失败，
  全部恰1test；文档冻结68输入SHA稳定，doc-exact/20261015T104229783610Z/summary.json。
- 本机ignored证据在 `target/teaching-v100-format-2026-10-15/`：旧binary基线
  `baseline-20261015T103007995206Z/summary.json`；真实unit/旧测试
  `runtime/20261015T103511605210Z/summary.json`；练习套解
  `exercises/20261015T103500250973Z/summary.json`（36commands）；合同
  `contracts/20261015T103838922136Z/summary.json`（12commands/228rawcase）；
  全量 `full-validation/20261015T104005353752Z/summary.json`（18commands/冻结
  脚本SHA）。full初始化103937579774Z只因路径假设错误停止、0命令，provenance-note
  保留，未重复跑gate。最终范围、HTML、图和文档exact见final-verification.json。
- 20版本教学检查、mdBook、38页 **1,383站内链接/0broken**通过。两SVG由bundled
  sharp/libvips渲染PNG并直接检查文字/箭头/页脚；Fontconfig缓存不可写告警不影响
  渲染。HTML另验include与图片引用，不把PNG审阅冒称浏览器页面视觉验证。
- 基线74fe的 [mdBook 38718514453](https://github.com/YanqingXu/lua_rust/actions/runs/38718514453)
  成功；[Rust 38718514632](https://github.com/YanqingXu/lua_rust/actions/runs/38718514632)
  completed/failure，两个Miri/Linux ASan成功；Windows仅M1质量门因生产fetch_update
  弃用的严格Clippy失败，其余测试/文档/audit门通过，official source三方10/10。
  baseline-book/rust.json与真实baseline-rust-job.log保留；本批不处理生产CI。

下一工作包从D005剩余转换或pattern选具体失败fixture继续推进，先定义边界再改实现
和证据，保留本轮signed i32/有限值/CRT差异的归属。发布证据治理、getfenv/setfenv、
TTY/signal/增量parser另立问题链。原审计仍暂停在os.date/os.time本地时间、UTC、DST，
本交接不表示恢复；本批按用户明确指令提交推送，新提交CI须按其SHA另查。

## 2026-10-15：结构化 REPL 协议迁入 v1.0 独立快照

用户要求将 v0.18 的结构化续行与 EOF 协议显式迁入 v1.0，复用字节验收合同，补齐
反例、前后差异、状态图和练习。本批教学、真实实现与验证已完成，**没有恢复
暂停的原审计目标**。用户随后明确要求将本批协议迁移、共享字节合同、章节、状态图、
练习包和交接记录一并提交并推送。

- 起点为已推送 `main@7776fd4f88ed9f2222545a5ece8375c0f9836aa4`，开始时工作区
  干净、origin/main 跟踪同 SHA。本批只修改 v100 本地 compiler、Repl library、
  main adapter、Cargo 两个 test target 与一行 capability evidence；新练习/示例/
  集成合同随快照自含，无新依赖。v18 只增三份共享测试文件并更新 README 计数；
  原 compiler/src/旧 exercises 不改。生产根 crates、两快照 core/VM/bytecode、
  v100 原兼容偏差、oracle manifest、2026-08-21 differential baseline 保持原样。
  提交前fetch确认HEAD与origin/main无分叉。本批提交号以
  `git log -1 -- book/src/versions/v100-lua51-runtime.md` 为准，新提交CI按其SHA查询。
- [主章](book/src/versions/v100-lua51-runtime.md) 新增三问：独立快照不会继承修复
  → 分类/controller/adapter 小步迁移；谁保存 source/诊断与 VM → EOF take一次；
  两边相同仍可都错 → 固定 byte oracle、独立执行与身份检查。每问有预测、固定反例、
  最小diff、源码include、exact验收；两幅新SVG对应 owner/输入状态与合同/证据链。
  v18 章末改为已迁移的下一章入口，历史批次的证据和完成计数不重写。
- 本地 CompileErrorKind/lexer/parser 与 Repl/run_repl 显式移植，保留 v1.0 括号
  prefix expression、string/table call sugar、Action::Help 正常顺序、arg/actions、
  v1.0 banner 及 JSON/trace v1。compiler 从7项到15项：共享分类6项、本版前缀与
  语法糖2项；分类仍是局部提示，每次重编译完整buffer，不证明任意前缀可补全。
- Repl 借用同一 Session，拥有 source 与最近一次 pending 诊断。未完成前缀零执行，
  成功编译后执行一次；执行返回/普通错误后清当前输入，不重建 VM 或回滚副作用。
  idle EOF=0；pending EOF clear/take原诊断一次、顶层打印并exit1，不重编译/执行。
  read_line正数提交实际片段，有LF才补LF；读取Err独立Host错误。非UTF-8 read_line
  仍失败；泛型writer只接REPL输出，Lua print走进程stdout，TTY/signal未扩展。
- [迁移练习包](book/code/v100-lua51-runtime/exercises/repl_migration/README.md)：
  一个finish TODO，6参考、6真实controller/I/O回归；固定faults为关键词漏判、静默
  EOF、clone重复诊断，example all有12条unique关键输出。PendingInput由本地真实
  compiler生成原诊断，教学投影不接Session；约束禁止重编译/执行，不宣称类型能
  禁止任意全局调用。TODO已验D/R初始失败→临时套解通过→原字节恢复并刷新mtime→
  再失败，交付仍保留TODO。原v18的十二题仍按历史版本归属，不另称本次套解。
- 永久[字节合同](tools/repl_byte_contracts/README.md)：canonical cases.tsv与两版
  本地fixture逐byte相同；前21条与上一批oracle程序逐byte继承，新增5条CRLF/无LF。
  两版wrapper/安全runner也相同，各自符合固定expected双流/exit，仅显式映射版本
  banner，不trim、不有损解码、不normalize。root Python gate检查schema/副本并可
  独立运行、独占归档stdin、actual/expected双流、metadata与SHA；永久入口随clone
  交付，本机target仅为本次证据。该26条不宣称官方完整TTY或任意进程树治理。
- 旧行为不是猜测：从HEAD7776 git archive提取独立旧v100、全workspace **136通过**
  并build后捕获3个真实case。表达式续行漏判造成两诊断；pending EOF静默exit0；
  无LF `return 'broken` 被旧adapter合成换行，实际newline in short string后exit0，
  不能误写成静默pending。证据baseline-capture.json保留原stdin/双流。初版验证器
  误猜第三条旧行为，仅修ignored预期并复用已通过build/test日志，原事实没有覆盖。
- Windows Rust/cargo **1.96.1**，v100本地release_gate一次运行通过fmt、all-targets
  Clippy `-D warnings`、D/R workspace，各 **158通过/0失败/1ignored**；v18 fmt、
  严格Clippy和D/R各 **215通过/0失败/12ignored**。v100=旧136+compiler8+协议6+
  参考6+共享2；v18=旧213+共享2。example每profile12条unique关键断言，共24次，demo各100；
  Help顺序/后续非法参数(code2)、v1.0文字、38opcode、JSON bytes/schema、CFG边端、
  trace schema与执行前时点均另核。官方双oracle本轮明确skipped，未冒充新差分。
- 两版D/R的Rust集成合同共104个真实case；独立Python另跑 **104个raw case**
  （52/profile），全部对固定oracle并回收。负控：副本漂移、同改三份为非法大写hex
  都被永久CLI拒绝；两个正确版本banner但固定idle输出的fake binary共52场景仅2条
  idle通过、50条拒绝，和真实104条分开；synthetic timeout已kill/reap且判失败。
  全workspace自然再跑集成case，未把重复执行包装成新增覆盖。
- 新v100独立copy只有本地源码，6packages全部path依赖闭合；全新target不共享产物，
  offline/locked build workspace bins、本地2项byte测试/26cases及demo100通过。
  继承祖先Rust linker配置，未复制v18或根tools源码。全量197输入SHA前后无漂移；
  文档去重17条exact，D/R共34命令，32正向通过/2learner预期失败，全部恰1test。
- 证据独立在 `target/teaching-v100-repl-2026-10-15/`：全量
  `full-validation/20261015T095858499820Z/summary.json`（30commands、冻结脚本SHA）；
  compiler局部 `compiler/20261015T094956108997Z/summary.json`；练习套解
  `repl_migration/20261015T095303694663Z/summary.json`；合同
  `contracts/20261015T095410846258Z/summary.json`，永久CLI负控
  `contracts/negative-controls-20261015T095526391391Z/summary.json`；文档exact
  `doc-exact/20261015T100230475936Z/summary.json`。最终范围/HTML/hash/图见
  final-verification.json。各层证据引用真实路径，不覆盖v18历史批次。
- 20版本教学检查、mdBook、38页 **1,363站内链接/0broken**通过。两幅SVG经bundled
  sharp/libvips渲染PNG实看文字/箭头/页脚；Fontconfig缓存不可写告警未影响中文。
  HTML另验include展开/两图引用；没有把PNG检查声称为浏览器页面视觉验证。
- 基线7776的 [mdBook 38714986340](https://github.com/YanqingXu/lua_rust/actions/runs/38714986340)
  成功；[Rust 38714986294](https://github.com/YanqingXu/lua_rust/actions/runs/38714986294)
  completed/failure，两个Miri和Linux ASan成功，Windows仅M1质量门失败。实际job
  日志确认生产fetch_update弃用触发严格Clippy失败，其余测试/文档/audit门通过，
  本SHA official source三方均10/10。baseline-rust-final.json与baseline-rust-job.log
  保留；未修改生产CI，此结果不代表本批新提交的远端CI。

下一任务应从v1.0 capability/deviation的具体失败fixture推进兼容收口，让每条差异
有反例、最小改动与oracle证据；TTY编辑、signal恢复、增量parser与生产CI工具链
弃用兼容另立范围。原审计仍暂停在os.date/os.time本地时间、UTC、DST，交接不代表
恢复；本批按用户明确指令提交推送，新提交CI须按其SHA另查。

## 2026-10-15：v0.18 结构化 REPL 续行与 EOF

用户要求按问题链讲清 NeedMoreInput、普通语法错误与未完成输入，补齐可运行反例、
前后差异、状态图和验收，完成验证并更新交接。本批教学与验证已完成，**没有恢复
暂停的原审计整改目标**。用户随后明确要求将本批章节、状态图、练习包和交接记录
一并提交并推送。

- 起点为已推送 `main@d4850d0b73039c73f07d7be823bd93215956c2a6`（上一批 runner），
  开始时工作区干净，本地跟踪 origin/main 同 SHA。本轮确实改了 **v0.18 快照** 的
  compiler/src/lib.rs、src/lib.rs、src/main.rs，并新增 tests/repl_input_protocol.rs；
  不把教学模型的理想行为冒充真实 binary。生产根 `crates/`、快照 core/VM/bytecode、
  旧 tests、旧 process/temp helpers 和 runner/工具包均无 diff。Cargo 仅增四个 test
  targets，没有新依赖或第二套 parser/VM。旧 recovery 仅迁移 real.rs 两个进程预期
  与 README；其 model/starter/faults/solution/contracts 保留历史启发式。
  提交前 fetch 确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v18-cli-tools.md` 为准，新提交 CI 按其 SHA 查询。
- [主章](book/src/versions/v18-cli-tools.md) 新增问题十一至十三：关键词漏判 / EOF
  位置误判 → 具体词法/语法位置给 kind；探测性重复执行 → compile成功才执行一次；
  静默 pending EOF / clone重复诊断 → 清buffer、take原诊断一次。每问有预测、反例、
  最小diff、源码include、图与exact验收。新增 classification/submit/eof 三图，旧
  recovery图显式标“迁移前”；正文与快照README统一为11包/12TODO，旧证据不重写。
- compiler 保留 `compile(&[u8])->Result<Proto,CompileError>`，新增公开
  `CompileErrorKind::{Invalid,NeedMoreInput}` 与 CompileError.kind；Display仍为
  原位置与message。构造错误的Rust调用者需提供kind，这是快照API变更。具体期待
  表达式/名称/闭合符的EOF为NeedMoreInput；错误token、畸形数字、escape>255、短串
  未转义换行、codegen语义/资源错误为Invalid。long string/comment与合法escaped
  newline可以续；lexer延迟unfinished literal，让更早确定语法错误先返回。
  新6项compiler测试覆盖18处parser缺口、六组跨行补全、数字/escape EOF、短/长串、
  delimiter层级与错误优先级；compiler总13项。NeedMoreInput是局部构造提示，**不是
  任意前缀都有合法后缀的证明**，每次重编译完整buffer，尚非增量parser。
- 真实 `Repl<'a>` 借用同一 `&mut Session`，拥有source与最后一份pending诊断。
  submit返回Blank/NeedMoreInput/Values或AppError；未完成chunk不执行前缀，成功
  编译只执行一次并清source/pending；普通编译/runtime错误清当前输入并继续，保留
  Session与既有副作用。空闲时只剥单个shorthand `=`，`==42`不再像旧
  trim_start_matches那样被重复剥离；pending内不改写。新6项实际协议集成测试
  检查零前缀执行/恰一次、错误恢复/owner、shorthand、短串、I/O Err、bytes/Nil。
- `run_repl`泛型BufRead/Write供注入I/O错误，CLI只adapter。read_line的Ok(0)才是
  EOF，Err保留Host错误；有数据但无LF的末片段按原bytes提交，不合成换行。公开
  submit表示完整物理行，内部submit_fragment接terminated。实际 `return 'broken`
  无LF为NeedMoreInput后pendingEOF=1；同bytes加LF先Invalid诊断、清buffer，闲时
  EOF=0。完整无LF `=40+2`仍成功。普通行错误显示后继续，最终闲时EOF可0；pending
  EOF先clear再take原CompileError，返回顶层打印一次并code1，不重编译/执行。
  重复finish不重复报错；输入Err不是旧pending诊断。泛型writer只承接REPL输出，
  Lua print仍走进程stdout，不作全局捕获；非UTF-8 read_line仍是Host错误。
- [分类包](book/code/v18-cli-tools/exercises/repl_classification/README.md)：一个
  decide TODO、7参考。共享借用真实Result<Proto,CompileError>，选择Execute/
  Preserve/Reject；实际compiler prefix/suffix与VM结果对照，关键词/EOF位置fault，
  只改message不改kind的措辞注入；4个真实-i子进程覆盖表达式、数字、短串、long
  comment，21条固定关键输出。不是另做缩小版parser或从诊断反推kind。
- [EOF包](book/code/v18-cli-tools/exercises/repl_eof/README.md)：一个finish TODO、
  5参考。PendingInput持source+Option<CompileError>，接口不提供现有Session/VM；
  练习约束禁止重编译/执行，不将类型说成能禁止调用任意全局函数。fixed faults
  静默丢诊断、clone不消费，合同检查原诊断/清source/重复EOF；实际Repl探针和10个
  CLI case另验owner、副作用与raw streams，固定all共15条关键输出。I/O故障是
  可注入BufRead合成故障，不冒充真实控制台硬件错误。
- 旧repl_recovery模型仍复现return40+漏判并保留原TODO；实际real.rs改为
  structured continuation无诊断输出两次42，以及pendingEOF一次诊断/code1，stdout
  尾换行保持。7参考重验通过；旧41条CLI marker只手写替换两条真实协议输出，其余
  39条沿用。旧README/图说明历史归属，不再声称当前真实CLI静默EOF或message判断。
- Windows Rust/cargo **1.96.1**：D/R workspace bins、fmt、workspace all-targets
  Clippy `-D warnings`、workspace tests通过，两profile各 **213通过、0失败、
  12ignored**（189+compiler6+实际协议6+分类7+EOF5）。两个新TODO已验证初始失败→
  临时套解通过→原字节恢复并刷新mtime→再失败，交付保留TODO。旧10题套解仍归前
  三批，本轮只回归其预期失败。11learner targets共12项预期失败，stdio两项。
- 11个CLI每profile169条unique关键断言，D/R共 **338条**；主章+11README去重
  **49条exact**各恰1test，37正向通过、12learner预期失败。独立直接启动lua_app的
  **21case/profile、共42个raw case**逐byte核对完整stdin/stdout/stderr/exit，含
  idle/pending/无LF/普通错/one-shot不续行/前缀零执行/成功恰一次/binary值；不只
  相信example自身检查。保留工具20项strict与两profile归档strict，另加两组REPL
  strict，共 **24组**；D/R export各6cases/36files，旧档案原bytes/metadata/SHA
  不变。demo1818、stdlib1550；PATH `D:\lua51\lua.EXE`实测Lua5.1.5并运行本版
  fixture1550，不把本轮REPL退出策略冒称完整官方REPL等价。136个代码输入hash
  前后无漂移。20版本教学检查、mdBook、38页 **1,339站内链接/0broken**通过。
- 证据独立在 `target/teaching-v18-repl-2026-10-15/`，最终
  `full-validation/20261015T092822333723Z/summary.json`：161命令、24strict、
  source/oracle/hash、冻结脚本副本与SHA。原run092719965834Z的26条通过命令复用；
  验证器把已清buffer后的新未闭块诊断误按整个stdin累计成4:1，实际正确2:1，仅修
  ignored脚本并续跑，原raw facts保留。092809878557Z是相对resume路径初始化失败，
  无命令执行，provenance-note明确归属。局部compiler为
  `compiler/20261015T091910851352Z/summary.json`，分类为
  `repl_classification/20261015T092012643239Z/summary.json`；EOF最终为
  `repl_eof/20261015T092433605192Z/summary.json`，092206994330Z属于修无LF前的
  中间版本，不当最终证据。initial-new-repl.json为已重建的新实现观察，非旧基线
  binary；各批历史证据没有覆盖。范围/hash/HTML/图见final-verification.json。
  target本机ignored；永久入口是主章、两个README、contracts/real、examples和源码。
- 图文验证：IAB隐藏预览两次webview attach超时，未声称页面视觉检查成功；改用
  bundled sharp/libvips把三个新SVG与历史recovery图渲染PNG直接检查，文字/箭头/
  状态框/页脚完整。Fontconfig仅有缓存目录不可写告警，渲染成功且中文实看；未改
  全局设置。生成HTML另验源码include/图片引用，无未展开标记；不把PNG检查说成
  浏览器页面布局验证。详情visual-review.json与render_svgs.cjs。
- 已推送基线d4850d0的 [mdBook 38711226895](https://github.com/YanqingXu/lua_rust/actions/runs/38711226895)
  **成功**，上一批教学helper lint修复已在该SHA验证。其 [Rust CI 38711226896](https://github.com/YanqingXu/lua_rust/actions/runs/38711226896)
  completed/failure：两个Miri与Linux ASan成功，Windows只在M1 foundation and
  quality gate失败。实际job日志确认生产fetch_update被新工具链弃用导致Clippy
  失败，其他测试/文档/audit门通过；本SHA的official source三方均10/10，不能套用
  更早883c28e的9/10。baseline-book.json、baseline-rust-final.json与真实
  baseline-rust-job.log保留；本轮未处理生产CI弃用问题。这些旧SHA结果不代表本批
  新提交的远端CI，须按新SHA查询，不能宣称已绿。

下一任务应将结构化协议显式移植到 **v1.0独立快照** 并复用相同字节合同；当前
v100/src/main.rs仍有旧is_incomplete，不会自动继承v18改动。其后按能力矩阵/oracle
逐项收口；TTY编辑/signal恢复/增量parser须另立范围。生产CI弃用兼容问题亦独立。
原审计仍暂停在os.date/os.time本地时间、UTC、DST；交接本身不表示恢复，提交推送
按当次明确指令。

## 2026-10-15：v0.18 fixture runner 与子进程生命周期

用户要求按问题链讲清 stdin EOF、stdout/stderr 并发读取、超时后 kill/wait 与证据
归档，补齐反例、前后差异、状态图和验收，完成验证并更新交接。本批教学与验证已
完成，**没有恢复暂停的原审计整改目标**。用户随后明确要求将本批章节、状态图、
练习包和交接记录一并提交并推送。

- 起点为已推送 `main@883c28eaf9f08dd6fd6f738661469906c24c1a7d`（上一批 v0.18
  JSON/CFG/trace），开始时工作区干净，origin/main 跟踪同 SHA。本批生产 `crates/`、
  v0.18 `src/`、`crates/`、既有 `tests/` 与旧 `exercises/support/` 均无 diff。
  Cargo 增六个 test targets 与受控 runner_fixture bin，显式 default-run=lua_app
  保持旧脚本入口，没有新增依赖。旧 JSON 教学 helper 另有下述已核实的 CI lint 修复。
  提交前 fetch 确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v18-cli-tools.md` 为准，新提交 CI 按其 SHA 查询。
- [主章](book/src/versions/v18-cli-tools.md) 新增问题七至十：write/flush 不等于 EOF
  → 关闭最后写端；先 wait / 顺序读 stdout 到 EOF → 两条输出同时 drain；返回超时 /
  kill-only → kill 后仍 wait；只写 bool / expected 污染 actual / latest 覆盖 → 原始
  capture 与预期分开、run/case 独占归档。每问含预测、真实固定反例、starter/solution
  include、最小 diff、状态图与精确合同。新增四幅 SVG 经浏览器检查文字、箭头与页脚；
  正文八段新 hook 展开，十种图片正常加载，无 unresolved include。
- [stdio 包](book/code/v18-cli-tools/exercises/runner_stdio/README.md)：两道独立 TODO
  send_input/start_drains、5 项参考。非空请求用 READY/ACK 证明首 byte 已读，再观察
  保留 writer 导致等待 EOF；空请求首次 read 在 EOF 返回0也会发 ACK，不能把 ACK
  一概说成非空消费。参考 write/flush/drop，完整 echo bytes 含 NUL/0xff，空输入
  同样验收。dual 子程序先写 1MiB stderr、后写 stdout，顺序读取/等 wait 后读取
  在握手后阻塞；参考逐 byte 匹配 stdout **1,048,596**、stderr **1,048,576** bytes。
  故障大块写被 kill 后可捕获空前缀；合同要求未完整且为精确前缀，不虚构固定部分量。
  主线程先 kill/wait，finish 再解除等待门并 join，固定故障不会遗留阻塞 reader。
- [timeout 包](book/code/v18-cli-tools/exercises/runner_timeout/README.md)：一个 stop
  TODO、6 项参考。窄 `&mut dyn Lifecycle` 注入 kill 竞态与一次 wait 失败，证明
  kill Err 仍尝试 wait，wait Err 保留 kill 上下文与 owner。两个 faults 在 hook
  返回边界观察，随后 backstop 安全收尾，不能以最终清理成功冒充 hook 正确。
  真实 stall READY 后 try_wait(None)，100ms 主 deadline 后终止回收，stdout22 /
  stderr17 原 bytes 保留；普通 exit0/7 属于 Exited，无 timeout hook/cleanup_status。
  try_wait(Some) 已回收，不再重复 wait；标准 Child Drop 不会自动 kill/wait。
  自定义 Guard 才有 backstop，writer lease 覆盖启动5秒+主timeout+清理5秒。
- [evidence 包](book/code/v18-cli-tools/exercises/runner_evidence/README.md)：一个
  archive TODO、7 项参考。真实 version、exit7、exit9、stall 四个子进程组成三份
  capture；判定分别 false/true/false，预期非零可以通过，timeout 的 cleanup code0
  仍不能当作普通成功退出。请求 stdin.bin、实际 stdout.bin/stderr.bin、另次探针的
  binary-version.bin、metadata.tsv、最后的 manifest.tsv 共六文件；path/argv/
  expected/status 文本用 hex，实测 elapsed.ns，actual termination 与 cleanup.* 分开。
  stdin 请求保存不证明全部消费；executable 路径/version 文本不等于二进制内容身份。
  run/case 用安全 ASCII，case create_dir、文件 create_new 拒绝覆盖；INCOMPLETE
  在最后成功才移除，I/O 错误保留不完整现场并返回 Err。reader 检查重复 key、完整行、
  schema/引用、跨文件/目录身份与长度，完整合同另对原 Capture 验实际 bytes；同长度
  篡改不能仅凭结构检查发现。明确非 atomic/fsync/断电恢复/身份认证协议。
- 新 fixture 无后代、输入输出有界；旧 helpers 保持原接口。三 CLI 不调用 learner，
  集成测试用 CARGO_BIN_EXE_runner_fixture，示例从 current_exe 找同 profile sibling；
  先 build --workspace --bins，Release 也先构建，不嵌套 cargo。并发读取保留各流
  顺序，不提供跨流全局时间线；deadline 不保证精确毫秒返回，且不声明孙进程/
  进程组/无限 I/O 的通用回收能力。成熟 tools/lua_fixture_runner 的 Job Object/
  process group、分类与规范化仍属独立能力。旧 GC 与 REPL 续行漏判归属不变。
- 本地 Windows Rust/cargo **1.96.1**：D/R workspace bins、fmt、workspace all-targets
  Clippy `-D warnings` 通过；两 profile 各 **189通过、0失败、10ignored**（原171+
  新18=stdio5/timeout6/evidence7）。四个新 TODO 完成初始失败→临时套解通过→原字节
  恢复并刷新 mtime 后再失败；保留 TODO。九 learner 显式运行共10项预期失败，stdio
  两项、其余各一项。此前六题套解证据仍归前两节，本轮只回归其初始失败。
- 九 CLI 每 profile133条 unique 关键断言，D/R共 **266条**；主章与九 README 去重
  **37条 exact** 各恰运行1项，27正向通过、10learner预期失败。保留旧工具20项 strict
  JSON/CFG/trace 检查，新增两 profile 的独立 Python 归档检查，共 **22项**：每个
  profile 各两次 export，共6cases/36files，核对完整 TSV、原 bytes、version、
  argv、请求stdin、elapsed、exit/timeout/cleanup、expected/verdict，旧文件哈希
  不变，另保存实际 runner_fixture SHA-256。demo1818、stdlib_diff1550；PATH
  `D:\lua51\lua.EXE` 实测 Lua5.1.5 并独立执行本版 fixture1550，未外推工具协议的
  官方等价性。117个代码输入验证前后 hash 无漂移；20版本教学检查、mdBook 与38页
  **1,327站内链接、0broken** 通过；生产核心/旧测试/helper 的无 diff 另作范围核对。
- 证据独立在 `target/teaching-v18-runner-2026-10-15/`。最终集成
  `full-validation/20261015T085746059881Z/summary.json` 含101项命令检查、22项 strict、
  oracle/hash；原始 `20261015T085611559102Z` 的19条通过命令复用，因验证器把 Windows
  扩展 canonical 路径 `\\?\G:\...` 与普通路径按字面比前缀而中断。仅修 ignored
  Python 路径规范化并续跑，原命令/交付代码未变、既有 export 未覆盖，summary、
  run_validation.used.py、脚本 SHA 与 provenance-note 保留来源。stdio 局部证据为
  `runner_stdio/20261015T085028265569Z/summary.json`（此前验证脚本把健康 stdout35
  误记45，保留旧证据并修正计数后续跑）；timeout 为 `runner_timeout/` 下 scoped-
  verification/starter-round-trip.json；evidence 为 `runner_evidence/20261015T085455070Z/`。
  最终范围/hash/站点/视觉汇总 `final-verification.json`。target 是本机ignored产物，
  clone 后永久入口是主章、九 README、contracts/real、examples 和 fixture 源码。
- 基线 SHA883c28e 的 [mdBook 38706177295](https://github.com/YanqingXu/lua_rust/actions/runs/38706177295)
  失败日志已通过 job connector 核实：CI Rust/Clippy **1.99.0** 在旧
  exercises/bytecode_json/model.rs 报 chunks-exact-to-as-chunks。已将经偶数长度校验
  的 chunks_exact(2) 换成 as_chunks::<2>().0.iter()，作为本批唯一旧教学代码修复。
  全 v18 D/R 回归通过，现有 **1.98.0** 工具链另定向 Clippy 通过（独立 target-dir，
  `ci-lint/20261015T085648458Z/summary.json`）；不声称本机运行过1.99或远端已绿。
  日志/状态为 baseline-book.json、baseline-book-job.log；先前 gh 下载403与实际
  connector 日志来源均保留。不要把旧节未知原因回填成这个新 SHA 的原因。
- 同一旧 SHA 的 [Rust CI 38706177480](https://github.com/YanqingXu/lua_rust/actions/runs/38706177480)
  已 completed/failure：两个 Miri 与 Linux ASan job成功；Windows失败步骤为 Enforce
  verified official source cases 和 M1 foundation and quality gate。真实 job log
  中 official source 为 Lua Rust9/10、官方Lua与固定C++各10/10；具体失败case本轮未
  进一步定位。quality gate 的 Clippy 因生产代码 fetch_update 被新工具链弃用而失败，
  其他测试/文档/audit门通过。该生产CI问题未在本轮教学包整改。终态与原日志分别
  baseline-rust-final.json、baseline-rust-job.log，早期 intermediate 另保留；这是
  已推送旧 SHA 的结果，不代表本批新提交的远端 CI；须按新 SHA 查询，不能宣称已绿。

下一教学工作包可单独推进结构化 REPL NeedMoreInput/EOF 合同；若进入 v1.0，先冻结
能力矩阵与 oracle 来源，再逐项收口差异。若优先恢复远端 CI，应另立范围处理生产
fetch_update 兼容性与 official source 失败case，不能用本章 runner 通过替代。原审计
仍暂停在 os.date/os.time 本地时间、UTC、DST；交接本身不表示恢复，提交推送按当次
明确指令执行。

## 2026-10-15：v0.18 字节码 JSON、CFG 与执行前 trace

用户要求继续按问题链讲清 bytes 常量编码、基本块 leader 划分、执行前状态记录，
补齐反例、差异、图与练习，完成验证并更新交接。本次完成该教学包，**没有恢复
暂停的原审计整改目标**。用户随后明确要求将本批章节、状态图、练习包和交接记录
一并提交并推送。

- 起点为 `main@af675e074231d9e0a1dd59ceede6521b36f1309e`（上一批 v0.18 CLI/REPL
  已推送），开始时工作区干净，本地跟踪 origin/main 同 SHA。本批生产 `crates/`、
  v0.18 `src/`、`crates/`、既有 `tests/` 与旧 support/process/temp helper 均无 diff。
  Cargo 仅增加六个 test targets，无新依赖。新增三个工具练习/CLI、一个 Lua fixture、
  一个定位 bytecode binary 的 wrapper；旧 CLI/REPL 包只回归，没有重新修改其实现。
  提交前 fetch 确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v18-cli-tools.md` 为准，新提交 CI 按其 SHA 查询。
- [主章](book/src/versions/v18-cli-tools.md) 问题四至六按三条原因/效果链推进：raw
  JSON 不合法 / lossy 碰撞 → lower hex 保留 bytes；漏 leader 把 jump 藏进块内 /
  offset 基准错 / 按源码行分块 → entry、target、控制流后继；after trace 时点错 /
  StackSlot alias 污染历史 → 先 clone 当前 Value 再 dispatch。每问有预测、固定反例、
  starter/solution include、最小 diff、图与精确验收。全章六包共六个独立 TODO；
  固定 faults 是人为错误设计，不冒充历史 VM，也不拼成第二套解释器。
- [JSON 包](book/code/v18-cli-tools/exercises/bytecode_json/README.md)：一个
  encode_bytes TODO、6项参考。借用 bytes 输出 string/hex schema；合同精确检查
  字段、小写 hex/偶数长度、解码等于原字节，覆盖空串、NUL、0xff、引号、反斜线、
  换行/UTF-8与lossy碰撞。真实手造 Proto 整文档核对13个 constants、nested Proto、
  code所有字段、NaN/inf/-inf字符串、metadata bytes escape；真实 binary stdin/file
  编译 nested fixture，完整 stdout 与 public renderer 比对，独立 Python strict
  JSON 再解码 nested bytes=00ff225c2f0a。没有新增 JSON parser 或序列化依赖。
- [CFG 包](book/code/v18-cli-tools/exercises/bytecode_cfg/README.md)：一个 leaders
  TODO、7项参考。使用真实 Proto/Instruction，框架固定半开块与末指令 edges；
  entry0、jump target=pc+1+sBx、jump或无fallthrough后pc+1，经 BTreeSet 排序去重。
  合同手写 starts/ranges/edges，完整 DOT 对照 public cfg_dot；forward/back、三种
  conditional、ForPrep/ForLoop、Return/TailCall、empty、invalid opcode、负/越界
  target均覆盖。固定漏切分/pc+sBx/按line分块可运行；真实compiler if精确6块4边、
  for精确5块4边，binary完整DOT一致。图中独立7指令示意经算法核对，五块四边；
  无入边的 B4 仍保留。只分析当前 Proto，不递归 nested，不做可达裁剪或执行保证。
- [trace 包](book/code/v18-cli-tools/exercises/bytecode_trace/README.md)：一个
  dispatch TODO、9项参考。极小3-op模型用真实 Instruction/StackSlot，四指令手造
  Proto 的 before 为 [nil,nil]→[40,nil]→[40,2]→[42,2]、top depth0/lines10..13；
  after把目的值提前，alias让历史全成[42,2]。真实VM另验手造/分支pc跳号但JSONstep
  连续、开关计数[0,8,0,4,0]、坏LoadK已录1event、非法opcode无event、预算2只录pc0/1。
  编译 return40+2 额外生成Move，为5events/3registers，与手造4events范围明确区分。
  真实CLI纯JSON五项用独立literal核对；runtime Err code1/stdout空而库前缀有7项；
  print成功则先写Lua输出再JSON。typed identity探针实证较早event不保留对象旧字段，
  仅由VM内部events持有的transient撤合法roots后GC，重新发布精确拒绝 StaleHandle。
- 编码/观察边界：bytes hex可逆不等于metadata文本保真；debug name/function按
  UTF-8 bytes转\u00xx，实际é解析成Ã©，未修renderer。非有限number是字符串
  NaN/inf/-inf，不保留NaN payload bits；没有Proto decoder/binary chunk或跨版本
  schema保证。CFG使用静态指令结构，不等于本次经过的路径。VmTraceEvent无step
  字段，trace_json对传入events枚举；budget/decode检查和快照borrow失败可先于event。
  enable清旧并开启，take转移Vec并关闭；clone Value不是Heap深拷贝或自动GC root。
  trace不能当undo/完整回放，CLI也不会在Err分支render partial JSON；纯JSON输入须
  避免Lua print污染。v0.15/v0.16 GC缺口与REPL续行漏判仍独立归属，未在本轮整改。
- 新 `examples/bytecode_views.lua` 返回42与bytes 00ff225c0a（NUL/0xff/引号/反斜线/
  换行），含if分支且无print；四format Debug/Release均实际运行，JSON精确解码，
  trace纯JSON有Add，DOT按工具发出的语法子集解析、指令pc全覆盖并校验edges存在。
  三幅新SVG经浏览器核对文字、箭头、半开范围/边标签与页脚，正文六段新hook正确
  展开、Lua fixture可见、三图加载，无 unresolved include。
- 复用旧进程runner（并发drain/EOF/五秒deadline/exact child kill-wait）和temp清理，
  新wrapper仅定位同profile的lua_bytecode。工具属于不同package，先运行
  `cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins`，
  Release同样先构建；不嵌套cargo，不从PATH选择其他工具。集成test从本次lua_app
  的CARGO_BIN_EXE路径找sibling。fixture有界无后代，旧进程树/大输出边界没有扩张。
- 本地Windows Rust/cargo **1.96.1**：D/R workspace bins、fmt、workspace all-targets
  Clippy `-D warnings`通过；两profile各 **171通过、0失败、6ignored**（既有149+
  新JSON6/CFG7/trace9）。三个新TODO各完成“初始1失败→临时套解1通过→原字节/哈希
  恢复并刷新mtime→再1失败”；旧三题套解证据仍归上一节，本次只重跑初始失败。
  六learner各1预期失败，交付保留TODO；默认ignored/固定CLI退出0不代表学习者完成。
- 六CLI每profile **112条unique关键断言**（旧74+JSON12+CFG10+trace16），D/R共
  **224条**；trace实际17行含一条重复Return观察，计数按去重marker。主章+六README
  去重 **26条exact** 各执行1项，20正向通过、6learner按预期失败。独立Python对
  **20项严格JSON/trace/CFG检查**拒绝重复key/裸非有限值，检查syntax/schema/原bytes/
  完整before与错误/print输出边界。demo stdout1818、stdlib_diff stdout1550；PATH
  `D:\lua51\lua.EXE`确认为Lua5.1.5，独立实际执行本版fixture1550，不冒充工具格式
  与官方Lua输出等价。85个代码输入前后哈希无漂移；20版本教学检查、mdBook构建、
  38页链接检查通过：**1,311站内链接、0broken**。
- 证据独立在 `target/teaching-v18-tools-2026-10-15/`：最终集成为
  `full-validation/20261015T082238632969Z/summary.json`，记录来源、计数、strict
  artifacts、oracle与hash。首段 `full-validation/20261015T082125709319Z/` 完成的
  9项build/fmt/clippy/workspace检查保留；脚本把17行trace误当17个unique marker时
  停止，随后修正target脚本去重计数，核对85源码hash一致后复用已通过检查继续，
  没有重跑这些检查或修改交付代码。最终summary的continued_from记录该归属。
  局部/套解为 `bytecode_json/20261015T081605406219Z/summary.json`、
  `bytecode_cfg/{scoped-verification,starter-round-trip}.json`、
  `bytecode_trace/20261015T081741036Z/summary.json`；最终范围/hash/站点/视觉见
  `final-verification.json`。这些为本机ignored产物；clone后永久入口是主章、各包
  README/contracts/real、examples及wrapper，没有覆盖以前版本证据。
- 已推送基线 `af675e0` 的 [mdBook 38703020482](https://github.com/YanqingXu/lua_rust/actions/runs/38703020482)
  成功；本轮末次观察 [Rust CI 38703020508](https://github.com/YanqingXu/lua_rust/actions/runs/38703020508)
  仍in_progress：三个Miri/ASan job成功，Windows job未终态，其M1 foundation and
  quality gate步骤已失败，具体原因未核实。明细为本证据目录
  `baseline-rust.json`/`baseline-rust-final.json`与`baseline-book.json`。这是旧SHA
  的CI，不是本批改动的远端结果，不套用上一提交的失败原因。提交前查终态：仅
  Windows Quality Gate 失败，失败步骤为 `M1 foundation and quality gate`，其余
  三个Miri/ASan job成功。失败日志下载返回HTTP 403 / API rate limit exceeded，
  具体原因仍未核实；明细与输出分别保存于 `push-baseline-rust.json`、
  `push-baseline-failed.log`。

本版六组问题链已完成；若继续教学，可从fixture阻塞/超时与证据归档推进进程
生命周期，或独立修订结构化REPL续行协议。若进入v1.0，按登记差异/能力矩阵与
oracle证据逐项推进，不以工具能输出JSON替代兼容验收。原审计仍暂停在
os.date/os.time本地时间、UTC、DST；交接本身不表示恢复，提交推送按当次明确指令。

## 2026-10-15：v0.18 CLI 错误退出与 REPL 跨输入状态

用户要求继续按问题链补齐 CLI 错误退出、REPL 跨输入状态的反例、差异、状态图和
验收，完成验证并更新交接。本次完成该教学包，**没有恢复暂停的原审计整改目标**。
用户随后明确要求将本批章节、状态图、练习包和交接记录一并提交并推送。

- 起点为 `main@dcf325bd2ea251c504c1c807739227b734ff3436`（上一批 v0.17 files 已推送），
  开始时工作区干净，本地跟踪 origin/main 同 SHA。本批生产 `crates/`、v0.18 快照
  `src/`、`crates/` 与既有 `tests/` 均无 diff；Cargo 只增加六个 test targets，无新
  依赖。新增教学代码位于三包、三个 Rust CLI 和两个共享 helper；没有修改真实 CLI、
  parser/VM 或旧进程 runner 的行为。
  提交前 fetch 确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v18-cli-tools.md` 为准，新提交 CI 按其 SHA 查询。
- [主章](book/src/versions/v18-cli-tools.md) 按三问组织：打印错误但进程成功 →
  Result/stream/exit code；每次 convenience API 创建新 VM → 借用同一 Session；
  每行清 source / 错误不清 / 首错退出 → 成功与普通错误清 buffer、未完成保留。
  每问均有预测、固定错误、starter/solution include、最小 diff、图、精确验收与
  owner/失败边界；修正原章把 message 匹配写成结构化错误类别的表述。bytecode
  JSON/CFG/trace 保留旧入口与原验收，没有扩成另一批工具实现任务。
- [退出包](book/code/v18-cli-tools/exercises/cli_exit/README.md)：一个 finish TODO、
  8 项参考。纯 Outcome→Transcript 模型接收已有 stdout 与 Result，不冒充捕获 Lua
  print；验 success=0/Usage=2/Runtime=1，diagnostic/usage，script return 不自动
  打印，已有输出不回滚。固定反例为只打印 Err 却 code0、全 Err code1、打印 return。
  17 个真实 child cases 覆盖 help/version、未知/缺 operand、compile/runtime/file、
  silent return、`-e/-l` 失败中止后续 action/script/-i、script 失败不进入 REPL。
- [owner 包](book/code/v18-cli-tools/exercises/repl_owner/README.md)：一个 submit
  TODO、5 项参考。直接用真实 Session，fresh 第二输入七项均 Nil；shared+GC 实读
  global/table/closure captured cell/package/registry，普通 chunk local 下一次 Nil；
  mutation 后再 GC 验字段与 loader 次数。独立公开 Vm probe 保留宿主 Value、撤真实
  roots、结束旧执行帧后 GC，再向 registry/global 写旧值精确拒绝 StaleHandle；另验
  registry 保活时实际读取 answer42，不把 Rust handle 副本当 root。真实 CLI 两个
  `-e → script` 与 `-e → -i` 分别检查跨输入字段、闭包计数、module 身份与完整输出。
- [恢复包](book/code/v18-cli-tools/exercises/repl_recovery/README.md)：一个 source
  lifecycle TODO、7 项参考。固定持久 Session，三故障分别丢 continuation、保留错误
  source 污染后续、首 Error 结束循环；合同检验 buffer/事件/下一输入值，runtime
  失败前 effects 已递增而不回滚。空闲空行、首行 `=`、空 Vec 与单 Nil 分别验收。
  8 个真实 child cases 覆盖 -i 编译/运行错后继续并最终 exit0、-e 错 exit1、返回
  数量、pending EOF、整段 stdin 编译与分行执行差异、启发式漏判后仍能继续。
- 实测当前续行边界：`return 40 +\n` 报 expected an expression，未命中三段 message
  规则，buffer 清空；追加 `2\n` 本可编译为返回42。该漏判保留为反例，没有假称
  已修完整 NeedMoreInput parser。pending EOF 不最终编译或另报未闭合，break 后局部
  source 析构并正常退出。无 script、无 actions 且未指定 -i 时，非终端 stdin 才
  默认 read_to_end；脚本 `-` 显式读 stdin，已有 -e 不自动再读管道。普通 REPL 行
  diagnostic 不等于最终非零退出，read/flush 等宿主 I/O 错误仍可能离开循环。
- Session 保留 Heap/global/package/registry/coroutine/resource owner，不保留普通
  chunk 词法作用域；每次 execute_multi 重置 call stack、活动帧/函数、temporary
  roots、manual GC mode 与指令预算。捕获的 local table 经真实闭包 upvalue 保活。
  任意 Value 逃逸、活动 varargs、sort snapshot、重入与所有 GC 路径均不由此证明；
  v0.15/v0.16 已登记 GC 缺口没有在本轮整改。
- 共享 process helper 并发 drain stdout/stderr，writer 写完 drop stdin 交 EOF，
  主线程五秒 try_wait deadline；超时只 kill/wait 自己的 child，然后 join，不把
  helper timeout 当成 Lua 自己的退出码。fixture 有界且不启动后代，不承诺通用
  process tree 或后代持 pipe 的 join 上界。旧 runner 仍先 wait 后读输出，未用其
  small-fixture 结果冒充大输出安全。临时目录唯一、canonical 范围核验、显式 finish
  合并清理错误、Drop fallback 不 panic；示例先 build 对应 profile binaries，按
  current_exe sibling 定位，不嵌套 cargo，也不从 PATH 偷换另一个 Lua。
- 三幅 SVG（exit/owner/recovery）经浏览器核对三路箭头、状态与页脚；正文六段新
  starter/solution、chunks/classifier 正确展开，三幅图加载，无 unresolved include。
  特别区分最终 Ok 可有 REPL stderr 诊断、EOF 局部析构、默认 pipe 的完整分支条件。
- 本地 Windows Rust/cargo **1.96.1**：Debug/Release 先 build --bins；fmt、workspace
  all-targets Clippy `-D warnings` 通过；两 profile 各 **149 通过、0 失败、3 ignored**
  （原 v0.18 129 + 新参考20）。这是独立快照，不将 v0.17 教学习题数重复计入。
  三个 learner 各初始1失败，均完成“失败 → 临时套解1通过 → 原字节/哈希恢复并刷新
  mtime → 再失败”；交付保留 TODO。默认 ignored 或固定 CLI 退出0不代表学习者完成。
- 三 CLI 每 profile **74 条关键断言**（exit25/owner8/recovery41），Debug/Release
  共 **148 条**；主章与三个 README 去重 **14 条 exact** 各执行1项，11正向通过、
  3 learner 按预期失败。demo 纯 stdout=1818，本版 stdlib_diff 纯 stdout=1550；
  PATH `D:\lua51\lua.EXE` 确认为 **Lua5.1.5**，独立实际执行本版 fixture=1550。
  差分范围是此确定性 fixture，不声称 REPL prompt/退出码与官方 Lua 全部等价。
  55 个代码输入（含两 helper）前后哈希无漂移；20版本教学检查、mdBook构建、38页
  链接检查通过：**1,299 站内链接、0 broken**。
- 证据独立在 `target/teaching-v18-2026-10-15/`：
  `full-validation/20261015T074853182144Z/summary.json`（36项检查、计数、oracle与
  hash），同目录逐项来源和日志；局部套解为
  `cli_exit/20261015T074537042289Z/summary.json`、
  `repl_owner/{scoped-verification,starter-round-trip}.json`、
  `repl_recovery/20261015T074649266Z/summary.json`。最终交付范围、哈希与站点/视觉
  核对见 `final-verification.json`。这些是本机 ignored 产物，clone 后的永久入口为
  主章、各包 README/contracts/real、examples 和共享 support，未覆盖以前版本证据。
- 已推送基线 `dcf325b` 的 [mdBook 38699535934](https://github.com/YanqingXu/lua_rust/actions/runs/38699535934)
  成功；本轮末次观察 [Rust CI 38699536098](https://github.com/YanqingXu/lua_rust/actions/runs/38699536098)
  仍 in_progress，三个 Miri/ASan job 成功，Windows Quality Gate 尚在运行。初查与
  末次明细分别是 `baseline-rust.json`、`baseline-rust-final.json`；mdBook 明细为
  `baseline-book.json`。这是旧提交 CI，不是本批改动的远端验证结果，也不套用上一
  SHA 的失败原因。提交前查终态：仅 Windows Quality Gate 失败，失败步骤为
  `Enforce verified official source cases` 和 `M1 foundation and quality gate`，
  其余三个 Miri/ASan job 成功。失败日志下载返回 HTTP 403 / API rate limit exceeded，
  具体原因未核实；明细与输出分别保存于 `push-baseline-rust.json`、
  `push-baseline-failed.log`。

若继续本版教学，可按“bytes 常量为何不能直接输出 JSON 文本 → CFG 为何按指令找
leader → trace 为何记录执行前状态”推进工具问题链；CLI/REPL 这三问已完成，后续
不要重复生成同一包。功能上修复续行漏判、原有 GC 缺口可另按明确指令启动。原审计
仍暂停在 os.date/os.time 本地时间、UTC、DST，交接本身不表示恢复。

## 2026-10-15：v0.17 文件 userdata、close/tombstone、GC prune 与 Drop drain

用户要求继续按问题链讲清 Lua 对象可达性与宿主句柄释放，补齐反例、差异、状态图、
验收并更新交接。本次完成该教学包，**没有恢复暂停的原审计整改目标**。用户随后
明确要求将本批章节、状态图、练习包和交接记录一并提交并推送。

- 起点为 `main@9171980d4913cd65b60436d67f83496915a55e1e`（上一批 require 教学包），
  开始时工作区干净，本地跟踪 origin/main 同 SHA。生产 `crates/`、v0.17 快照核心
  与既有 `tests/` 无 diff；Cargo 仅增加六个 test targets，无新依赖。本批新增代码
  位于三个 exercises、三个 Rust CLI 与一个 Lua smoke，旧 require 包仅回归验证。
  提交前 fetch 确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v17-host-stdlib-modules.md` 为准，新提交 CI 按其 SHA 查询。
- [主章](book/src/versions/v17-host-stdlib-modules.md) 新增问题四至六：flag-only
  仍持有 File / remove 丢失 closed 身份 → take 留 None；Heap 已 stale 而 map 仍
  持有 File → completed 周期后按 is_live retain；全局仍可达的文件在 VM owner
  结束时 → files.clear 先于 Heap shutdown。每问有预测、固定反例、starter/solution
  include、最小 diff、图、精确验收与迁移。现在六包共七个 TODO，互相独立，不拼接
  成第二套完整 VM；固定 faults 是人为设计，不冒充历史中间版本。
- [close 包](book/code/v17-host-stdlib-modules/exercises/file_close/README.md)：一个
  TODO、8 项参考。真实 Heap + `HashMap<UserdataRef, Option<OwnedFile>>`，内部实际
  File，登记消费 owner，无 handle clone。合同读写 `A\0\xff`，验一次释放、None、
  alias、foreign userdata 的 NotFile/Type 与 closed 的 Runtime。真实 VM 单独验
  io.close/方法 close 两方向别名、12 项错误矩阵、关闭后独有 metatable metadata
  跨 execute/host GC 实读 42，以及 VM/closed userdata 仍活着时 OS File 已释放。
  模型 foreign userdata 与真实 table 误传的错误消息路径已明确分开。
- [prune 包](book/code/v17-host-stdlib-modules/exercises/file_prune/README.md)：一个
  TODO、6 项参考。真实 Heap/File 四格（live Some/live None/stale Some/stale None）；
  budget=1 的 incomplete Sweep 已删第三项但尚未 completed 时，map 仍完整；完成
  Sweep 后先观察 stale userdata 与残留 File，再 retain。错误版漏 prune、按 Some
  retain、把全部 map key 加 roots 分别暴露宿主残留、身份丢失、自举保活。真实 VM
  多 execute 只向宿主返回 bool，再 host GC，弱槽精确 nil；活 open 实做 write/flush，
  活 tombstone 实做方法查找/closed error，撤全局 roots 后再验四项全部释放。
- [drain 包](book/code/v17-host-stdlib-modules/exercises/file_drain/README.md)：一个
  TODO、4 项参考。实际 Owner Drop 记录 mixed Some/None 在 shutdown 边界的状态：
  健康版先清空，boundary open=0；故障只 shutdown，body 内 open=2，但自定义 Drop
  返回后 Rust 仍自动析构字段，最终两版都是 drops=[1,1,1]。**故障证明顺序不同，
  不声称整个 owner 销毁后永久泄漏。** 真实 Heap 探针覆盖 reachable userdata 与
  pending finalizer 队列被 shutdown 清空，getter/分配精确拒绝为 Shutdown；真实 VM
  全局 open file 跨 execute/host GC 实读 owned，VM 作用域结束后重开仍读 owned。
- 修正原章“Windows 删除成功即证明关闭”的过强表述：Unix 可 unlink 打开文件，
  Rust Windows 默认共享方式也可能允许删除。三个新包用 Windows `share_mode(0)`
  实际独占重开，先验证仍持有时被阻挡，再验证 close/prune/Drop 后成功；prune/drain
  严格检查错误码 32，close 接受 sharing/lock 32/33，其他错误失败。非 Windows 明示
  无同等 OS 证据，普通重开、删除与 Drop 计数均不冒充通用内核句柄检测。清理用唯一
  临时目录、canonical 范围核对、显式 finish 与不 panic 的 Drop fallback。
- 源码边界：take 只移动 owner，移出值析构才释放；真实 close 的临时 Option 在成功
  返回前 drop。files keys 不属于 gc_roots，file metatable 则是 root/Heap tracing
  edge。prune 根据**先捕获的 step.completed**，在 finalizer events 保活/调用后、
  临时 roots truncate 与首 callback 错误返回前运行。is_live 检查记录是否存在，
  未 Sweep 的白对象仍可能为 true；不声称提前 retain 必然误删。pending finalizer/
  resurrection 可使 userdata 继续 live，四格无任意 finalizer 组合覆盖。Heap shutdown
  不执行 Lua finalizer；File Drop 不报告关闭错误、不承诺 fsync 或 exit/abort 清理。
  v0.15 active varargs、v0.16 sort snapshot roots 等历史缺口仍未在本轮修复。
- 三幅 SVG（ownership、prune 四格、drain 顺序）经浏览器检查文字、列、箭头和页脚；
  正文六段新 starter/solution 都正确展开，三幅图均加载，无 unresolved include。
  新 `file_lifecycle_contracts.lua` 返回 true，验证显式 close 与别名；末尾删除仅清理。
  原 demo=1717、require_contracts=true、快照 stdlib_diff=1550。
- 本地 Windows Rust/cargo **1.96.1**：fmt、workspace all-targets Clippy `-D warnings`
  通过；Debug/Release 各 **161 通过、0 失败、7 ignored**（既有 143 + 文件参考 18）。
  六 learner 初始分别 2/1/1/1/1/1 失败；三个新 TODO 各完成“失败 → 临时套解通过 →
  原字节恢复后再失败”，starter 哈希一致并刷新 mtime。旧四题闭环证据仍归上一节，
  本轮只重跑其初始失败，未伪称重新套解。ignored 或 CLI 退出 0 不等于 learner 完成。
- 六 CLI 共 **98 条关键断言**（旧 55 + close 19/prune 8/drain 16）；主章与六个
  README 去重 **33 条 exact** 各执行 1 项，26 正向通过、7 learner 按预期失败。
  PATH `D:\lua51\lua.EXE` 确认为 **Lua 5.1.5**，独立实际执行本版 stdlib_diff=1550；
  77 个代码输入验证前后哈希无漂移。20 版本教学检查、mdBook 构建、38 页链接检查
  通过：**1,285 站内链接、0 broken**。
- 证据独立保存在 `target/teaching-v17-files-2026-10-15/`：集成验证为
  `full-validation/20261015T071748258547Z/summary.json`（58 项检查、计数、oracle、
  哈希），同目录 exact 来源与日志；局部/套解为
  `file_close/20261015T071242118965Z/summary.json`、
  `file_prune/{scoped-verification,starter-round-trip}.json`、
  `file_drain/20261015T071539870Z/summary.json`。`final-verification.json` 记录最终
  交付哈希、范围、站点和视觉核对。这些是本机 ignored 产物，clone 后以主章、各包
  README/合同/real 和 examples 为永久复现入口；未覆盖以前证据。
- 已推送基线 `9171980` 的 [mdBook 38696072191](https://github.com/YanqingXu/lua_rust/actions/runs/38696072191)
  成功；本轮末次观察 [Rust CI 38696072267](https://github.com/YanqingXu/lua_rust/actions/runs/38696072267)
  仍 in_progress：ASan 与 GC/Native Stack Miri 成功，Unsafe Durability Miri 和 Windows
  job 尚在运行；Windows 的 `Enforce verified official source cases` 步骤已失败，具体
  原因未核实。保存于本证据目录 `baseline-{rust,book}.json`；这是旧提交的 CI，不是
  本批教学改动的远端结果，不套用上一 SHA 的失败原因。提交前再查终态：仅 Windows
  Quality Gate 失败，失败步骤为 `Enforce verified official source cases` 和
  `M1 foundation and quality gate`，其余三个 Miri/ASan job 成功。失败日志下载返回
  HTTP 403 / API rate limit exceeded，具体原因仍未核实；明细与输出分别保存于
  `push-baseline-rust.json`、`push-baseline-failed.log`。

若继续本版教学，可从 IO 的空字节串/EOF/读取失败区别推进 bytes 与 Result 协议；
若进入下一 checkpoint，v0.18 可按 CLI 错误退出与 REPL 跨输入状态推进。文件 owner
三问已完成，后续不要重复生成同一包。工程 GC 缺口可按独立指令整改；原审计仍暂停
在 os.date/os.time 本地时间、UTC、DST，交接本身不表示恢复该目标。

## 2026-10-15：v0.17 require 缓存、循环加载与失败恢复

用户要求继续按问题链补齐这三部分的可运行反例、代码差异、状态图与验收，完成验证
并更新交接。本次完成该教学包，**没有恢复暂停的原审计整改目标**。用户随后明确
要求将本批章节、状态图、练习包和交接记录一并提交并推送。

- 起点为 `main@93bfbfef5ff4a92d87fac2b61b098475709f61d1`，开始时工作区干净，
  本地跟踪的 origin/main 同 SHA；这是上一批已推送的 v0.16 callbacks 教学包。
  本次生产 `crates/`、v0.17 快照核心与既有 tests 均无 diff；Cargo 仅增加六个
  test targets，未增加依赖。历史证据与功能缺口保持各自版本归属。
  提交前 fetch 再次确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v17-host-stdlib-modules.md` 为准，新提交 CI 须按其 SHA 查询。
- [主章](book/src/versions/v17-host-stdlib-modules.md) 按三问组织：每次执行/存在性
  判断 → truthy cache 与首结果提交；晚发布导致 A→B→A 重入 → 调 loader 前发布
  true；`?` 留下假缓存/清全部破坏依赖 → 保存 Result、失败只删当前 key。每问有
  预测、固定运行入口、starter/solution include、最小 diff、图与验收。原文件/io/os/
  debug 能力保留为 checkpoint 背景，没有把本轮扩展为宿主 API 或原审计整改。
- [缓存包](book/code/v17-host-stdlib-modules/exercises/cache/README.md)：两个独立
  TODO、9 项默认参考。模型使用真实 Value 的 BTreeMap，不包含 Heap/GC 或文件定位；
  检查 nil/false miss、0/空串 hit、只消费首结果、loader 自发布与 false 再加载。
  真实 VM 另核对 15 行矩阵、正常/自发布表的身份与字段；跨 execute 只保留 bool，
  公开 package.loaded 字段重绑后 host GC，再 require 实读 answer=42、calls=1，
  新公开表仍无本项，验证 VM 初始私有 loaded owner 与字段不同。
- [循环包](book/code/v17-host-stdlib-modules/exercises/cycles/README.md)：一个
  publication TODO、6 项默认参考。真实 Heap 的局部模型使用有限 loader 进入预算，
  晚发布自循环进入六次，A/B 各三次后终止；健康各一次并验精确 trace/最终表字段与
  身份。预算只为反例有界，不是正式循环错误。真实 preload 与临时文件都验证内层
  看到 true，最终 A/B 为表；显式 partial table 与完成 true 也沿相同缓存分支。
  循环模型只处理固定单结果成功路径，不承担完整提交矩阵或失败恢复。
- [重试包](book/code/v17-host-stdlib-modules/exercises/retry/README.md)：一个
  attempt TODO、9 项默认参考。固定 `?` 故障使修复后返回旧 true、loader 不再运行；
  clear-all 故障使成功 dependency 与 prefix 丢失。合同检查原 VmError 种类/位置/
  消息、当前项精确 nil、依赖与 prefix 保留、副作用不回滚；原 false/自发布值在
  loader 失败后都被删，不恢复旧值。真实 preload 同 execute 修 loader 重试，另验
  A 失败保留成功 B、require 拒绝 yield 后同 coroutine 修 loader 再合法 yield/resume。
  文件探针在各自同一 VM 中验证 missing/compile/runtime 三阶段修文件重试：标记前
  错误保留 false，运行错误后变 nil；首可读文件编译失败不会继续后模板。
- 三幅 SVG（命中/提交矩阵、A/B 时序、分阶段失败清理）与正文 include 已浏览器
  核对文字、箭头、列对齐和页脚。新增 `require_contracts.lua` 返回 true：正常模块
  只执行一次、循环内层见 true、失败重试实际执行两次且 effects=2；原 demo 返回 1717。
  临时文件 probe 使用唯一目录、范围验证与显式清理；retry helper 的旧记录
  `retry/20261015T064657490Z/` 属于清理校准前，保留但不作为最终 helper 的验证。
- 源码边界已校准：首结果非 nil 才写缓存（包括 false）；nil/无结果重读当前 slot，
  若 loader 已清 slot，仅返回 true 而**不写回**，下次仍会加载。true sentinel 没有
  独立 Loading 身份，不等于完整模块对象或初始化完成，也不冒充官方循环报错行为。
  VM 保存初始 loaded/preload 句柄为 roots，path 则从当前 global package 读取；
  结果到提交的现有同步路径没有额外 GC checkpoint。普通 loader Err 删除当前 key；
  若 Heap 清理本身失败，真实源码可用 cleanup 错误覆盖原错。模型不证明 panic/abort、
  所有 GC 阶段或整个依赖图/OS 副作用回滚，v0.15/v0.16 既有缺口未在本轮整改。
- 本地 Rust/cargo **1.96.1**：fmt、workspace all-targets Clippy `-D warnings` 通过；
  Debug/Release 各 **143 通过、0 失败、4 ignored**（119 原测试 + 24 新参考）。
  learner 三组显式初始分别 2/1/1 失败，四题均完成“失败 → 临时套解通过 → 原字节
  恢复后再失败”，starter 哈希一致并刷新 mtime。ignored 与固定 CLI 退出 0 均不代表
  学习者完成。46 个代码输入前后哈希无漂移。
- 三个 CLI 共 **55 条关键断言**匹配；主章和三个 README 去重后 **22 条 exact
  命令各执行 1 项**，18 正向通过、4 learner 按预期失败。PATH `D:\lua51\lua.EXE`
  确认为 **Lua 5.1.5**，实际执行本版 stdlib_diff 输出 1550，不借用旧版本外部证据。
  全 20 版本教学检查、mdBook 构建、38 页链接检查通过：**1,267 站内链接、0 broken**。
- 新证据独立在 `target/teaching-v17-2026-10-15/`：
  `full-validation/20261015T065334031787Z/{summary,metadata}.json` 记录 40 项验证/
  计数/源码哈希/oracle；`exact-command-sources.json` 记录逐题来源。局部与套解为
  `cache/20261015T064807075236Z/`、`cycles/{scoped-verification,starter-round-trip}.json`、
  `retry/20261015T064950743Z/`；`final-verification.json` 为最终文档、站点与交付
  文件哈希。目录为本机忽略产物，不随 clone 交付；永久入口是主章、三个 README、
  contracts/real 与 examples，没有覆盖以前版本证据。
- 已推送基线 `93bfbfe` 的 [mdBook 38693555628](https://github.com/YanqingXu/lua_rust/actions/runs/38693555628)
  成功；本轮观察 [Rust CI 38693555655](https://github.com/YanqingXu/lua_rust/actions/runs/38693555655)
  仍在运行。三个 Miri/ASan job 成功，Windows Quality Gate 尚未终态，其
  `Enforce verified official source cases` 步骤已失败，具体原因未核实。明细保存于
  `baseline-rust.json`。提交前再查终态：仅 Windows Quality Gate 失败，失败步骤为
  `Enforce verified official source cases` 和 `M1 foundation and quality gate`，
  其余三个 Miri/ASan job 成功。失败日志下载返回 HTTP 403 / API rate limit exceeded，
  未核实具体原因；明细和输出分别见同证据目录 `push-baseline-rust.json`、
  `push-baseline-failed.log`。不套用以前提交的原因，也不作为本批新提交 CI 结果。

若继续本版教学，可从 file userdata 不可达但宿主 map 仍拥有 OS handle 的问题，
拆解显式 close/tombstone、GC prune 与 Drop drain；若进入下一 checkpoint，v0.18
可按 CLI 错误退出与 REPL 跨输入状态推进。工程整改仍可独立处理 v0.15 活动 varargs、
v0.16 sort snapshot roots 等已记录缺口。后续按用户指令启动；原审计仍暂停在本地
时间/UTC/DST，提交推送由当次指令授权。

## 2026-10-15：v0.16 pcall/xpcall、sort comparator 与禁止 yield 边界

用户要求继续按问题链讲清 callback 返回/报错后的状态恢复与临时对象保活，补齐反例、
差异、图与验收并更新交接。本次完成该教学包，**没有恢复暂停的原审计整改目标**。
用户随后明确要求将本批章节、状态图、练习包与交接记录一并提交并推送。

- 起点为 `main@467a4d15d542c137468c11eab0d666a8f46c75fb`，开始时工作区干净，
  本地跟踪的 origin/main 同 SHA；这是上一批统一参数/集中注册/全局环境教学包。
  本次生产 `crates/`、v0.16 快照核心与既有 tests 均无 diff；Cargo 只增加六个
  test targets，未增加依赖。之前的验证与已知缺口保留各自版本归属。
  提交前 fetch 再次确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v16-core-stdlib.md` 为准，新提交 CI 须按其 SHA 查询。
- [主章](book/src/versions/v16-core-stdlib.md) 接续问题四至六：pcall 只取首值丢掉
  成功窗口 → 消费完整 Vec；xpcall 无条件 handler/展开全部 handler 输出 → 只处理
  Err 并选择首值；`?` 越过恢复/内层清零 → 保存 Result、恢复 depth 后交付；sort
  仅保当前 pair → 全 snapshot roots 作用域。每段有预测、运行入口、前后 include/
  diff、图和验收。新增 callback_contracts.lua 在同一 coroutine 内捕获禁止 yield，
  后续合法 yield 41,nil，resume 输入 42 后返回 43，主 chunk 返回 true。
- [保护结果包](book/code/v16-core-stdlib/exercises/protected/README.md)：两个独立
  TODO、9 项默认参考。区分空窗口/一个 nil/false，pcall 保留 7,nil,9,nil 的五值
  结果；xpcall 成功不调用 handler，失败恰好一次，首 nil 不过滤、空结果补 nil、
  handler Err 转其文本。模型只消费已取得的 Result；真实 VM 另验同 execute 内
  报错后继续、参数弱槽精确 nil、handler 返回 table 写回后 GC 仍能读取 answer=42。
  本快照 error payload 已字符串化，不能宣称保存任意原始 Lua 错误对象。
- [边界包](book/code/v16-core-stdlib/exercises/boundary/README.md)：一个 TODO、
  8 项默认参考。固定 `?` 泄 depth 与无条件设 1/0 的反例分别导致过度拒绝/提前
  放行并发布 pending99；合同检查初始 depth 0/3、Ok/Err/禁止 yield、嵌套
  1→2→1→0、拒绝时无 pending、退出后新 yield 17 不混旧值。五个真实 VM probe
  分别覆盖 pcall、xpcall body/handler、sort、callable __call、nested pcall，均
  在同一次 execute 的同一 coroutine 内继续合法 yield，再 resume 到 Dead。
  实际源码为配对 +=1/-=1，模型用 baseline 表达合同；普通 Result 不外推 panic/abort。
- [排序 roots 包](book/code/v16-core-stdlib/exercises/sort_roots/README.md)：一个
  TODO、5 项默认参考。使用真实 Heap 的独立快照算法实验：四项 rank 4/3/2/1，
  comparator 清源后 GC，当前 pair 仍可读而另两项访问失败；proposal 全组入根后
  排序回写成功。成功后清源撤根，四对象均实际失效；Err 场景第二次比较写 source[1]=99，
  恢复精确 prefix 后旧第 1 项回收，其他三项仍由 source 保活。prefix 仍可读 900，
  callback 数据修改不回滚。真实 VM 正向只验源边保持时 GC、两元素 false/nil/0
  truthiness 与 error mutation，不能据此称清源缺口已修复。
- 三幅新 SVG（保护结果分支、嵌套 depth、快照/pair roots）及正文 include 已在
  浏览器核对文字、框线、箭头与页脚。源码边界明确：handler 结果到 caller 写回之间
  现有路径无额外 GC checkpoint，测试在写回后才收集；通常 invoke 路径有 roots
  truncate，但 CoroutineWrapper 的 `resume_coroutine(...)?` 仍可能跳过该收尾，
  本包未修复或证明该分支。v0.15 active_varargs_gap 的功能缺口也没有被覆盖。
- **新增独立已知功能缺口，未修复：** [sort_snapshot_roots_gap.lua](book/code/v16-core-stdlib/examples/sort_snapshot_roots_gap.lua)
  从已结束的构造帧取得四个表，首 comparator 清空源表后 GC，后续比较实际报
  `line 14 (pc 27): stale GC handle for object 13`。Debug/Release 均退出 2；
  单列 `known-functional-failure / functionality_pass=false`，不进默认参考或 ignored。
  健康 proposal 只改教学实验，没有改 VM 的局部 values roots。本节要求来自所选
  Rust snapshot 算法的内部所有权合同，不承诺官方 Lua 对 comparator 修改源表时的
  最终排序结果，也不以此输入做 oracle 等价证明。
- 本地 Rust/cargo **1.96.1**：fmt、workspace all-targets Clippy `-D warnings` 通过；
  Debug/Release 各 **155 通过、0 失败、8 ignored**（前批 133 + 本批 22 参考）。
  六组 learner 显式初始分别 2/1/1/2/1/1 失败；本批四题均完成“失败 → 临时套解
  通过 → 原字节恢复后再失败”，starter 哈希一致且 mtime 刷新。默认 ignored 不代表
  学习者完成，CLI 也不读 starter。73 个代码输入前后哈希无漂移。
- 六个 CLI 共 **75 条关键输出**匹配（原三包 37、本批 38）；demo 1616、两个正常
  contracts Lua 探针 true。主章与六个 README 去重后 **28 条 exact 命令各执行 1 项**，
  20 正向通过、8 learner 按预期失败。PATH 的 Lua **5.1.5** 实际运行 stdlib_diff，
  退出 0、输出 1550；只代表该确定性 fixture 的外部对照。全 20 版本教学结构、mdBook
  构建、38 页链接检查通过：**1,247 个站内链接、0 broken**。
- 新证据独立在 `target/teaching-v16-callbacks-2026-10-15/`：
  `full-validation/20261015T062503047334Z/{summary,metadata}.json` 记录 48 项验证/
  计数/源码哈希/oracle，复用同输入 `20261015T062339879420Z/` 已通过的七项；早期
  protected CLI 检查器的两条引号 marker 失配保留原记录，校准显示格式后符合预期，
  没有改源或重新把已过 workspace 当作新一轮验证。`exact-command-sources.json`
  记录逐题来源。局部与套解证据为 `protected/20261015T062030978Z/`、
  `boundary/20261015T061914452186Z/`、`sort_roots/{scoped-verification,starter-round-trip,known-failure}.json`；
  `final-verification.json` 为最终文档/站点/交付哈希核对。这些是本机忽略产物，
  不随 clone 交付；永久入口是主章、三个 README、contracts/real 与 examples，
  没有覆盖前一批 `target/teaching-v16-2026-10-15/` 证据。
- 已推送基线 `467a4d1` 的 [mdBook 38690972897](https://github.com/YanqingXu/lua_rust/actions/runs/38690972897)
  已成功；本轮观察 [Rust CI 38690972904](https://github.com/YanqingXu/lua_rust/actions/runs/38690972904)
  仍运行，GC/native Miri 和 Linux ASan 成功，Windows quality 与另一 Miri 仍运行。
  明细保存于 `baseline-rust.json`；不把旧提交失败原因套用到这次，也不把基线 CI
  作为本批新提交的 CI 结果。提交前再查终态：仅 Windows Quality Gate 的
  `M1 foundation and quality gate` 失败，其余三个 Miri/ASan job 成功。此次失败日志
  下载仍返回 HTTP 403 / API rate limit exceeded，未核实具体失败原因；明细与输出
  分别见同证据目录 `push-baseline-rust.json`、`push-baseline-failed.log`。

若继续教学，下一包建议 v0.17 从 require 重复执行 → 缓存 → 循环加载 → 失败后的
缓存/资源恢复推进。若选择工程整改，先从上述 sort snapshot 实际失败增加回归，再修
roots 作用域，并保留成功/错误/清源/嵌套对照；另处理 v0.15 活动 varargs 缺口。
后续按用户指令启动；原审计仍暂停在本地时间/UTC/DST，提交推送由当次指令授权。

## 2026-10-15：v0.16 统一参数规则、集中注册与唯一全局环境

用户要求继续按问题链完善这三部分，补齐可运行反例、前后差异、状态图和验收练习，
完成验证并更新交接。本次完成该教学包，**没有恢复暂停的原审计整改目标**。用户随后
明确要求将本批章节、状态图、练习包和交接记录一并提交并推送。

- 起点为 `main@77f75763b8d552a0cf592fe706c021402b6a8815`，开始时工作区干净，
  本地跟踪的 origin/main 同 SHA；这是上一批已推送的 v0.15 教学包。本次生产
  `crates/`、v0.16 快照核心与既有 tests 均无 diff。Cargo 仅增加本地 lua_stdlib
  dev-dependency 和六个练习 test targets，lock 相应增加一个已有 workspace 依赖。
  提交前 fetch 再次确认 HEAD 与 origin/main 无分叉。本批提交号以
  `git log -1 -- book/src/versions/v16-core-stdlib.md` 为准，新提交 CI 须按其 SHA 查询。
- [主章](book/src/versions/v16-core-stdlib.md) 按三问展开：分散取参导致 coercion/
  错误编号不一致 → Args/HostError；已有函数体但漏安装/错 ID → catalog 驱动；
  假设给 Rust globals 复制 `_G` 后读写分叉 → 唯一 Heap global_env。每问有预测、
  固定反例、可编译 include、最小 diff、图、精确验收和下一问题。v0.15 实际尚无
  `_G` 安装，双存储明确是人为失败迁移，未当成真实历史。
- [参数包](book/code/v16-core-stdlib/exercises/args/README.md)：两个独立 number/
  bytes TODO、9 项默认参考；真实 Args 验证借用别名、owned Vec 脱离输入仍有效、
  缺参/nil/optional 默认、UTF-8→trim→Rust f64 词法边界；真实 VM 核对成功值与
  Type/函数名/一基编号。非法 byte 255 的故障版变成 239/191/189，参考保持原字节。
  不把 Args::table 的变体检查称为 Heap liveness 校验，也不宣称完整 Lua 数值词法。
- [注册包](book/code/v16-core-stdlib/exercises/catalog/README.md)：一个 installer
  TODO、8 项默认参考；独立 68 项名字/ID 清单检查目录和安装结果，额外以漏 math 库、
  错 sqrt ID 的声明变体证明清单不会随待测输入缩小。固定漏装 sqrt/错配 Abs 分别
  得到 nil/81，参考实际调用得到 9。同库重复拒绝、跨库同名可共存，后者也验实际调用。
  preflight 只属于教学实验，真实 Vm::new 没有该动态检查；gc_* 诊断仍另行安装。
  实验 Heap 读出稳定 ID，替换固定 proto 唯一 0 常量后交真实 VM 分派；真实 VM 自身
  初始化另有代表调用对照，没有把实验 Heap 注入 VM，也未新增全部 68 函数体的行为证明。
- [全局环境包](book/code/v16-core-stdlib/exercises/globals/README.md)：一个显式
  initializer TODO、7 项默认参考。参考实际分配表、写 seed/_G 自指，constructor
  只包装 handle；合同检查双向写、精确 nil 删除、_G 重绑、跨 GC 访问与撤字段回收。
  独立 Heap 撤去外部根后，自环 env/child 的 `is_live==Ok(false)` 且访问失败。
  真实 VM 分多次 execute/host GC，只返回 bool，weak owner/child 两槽精确 nil；
  另在 `_G` 与观察别名都置 nil 后检查裸 global 与 math 调用。VM 的根枚举保活环境，
  Copy handle 与自环都不自动成为根；替换 `_G` 字段不替换 Vm.global_env 身份。
- 三幅 SVG（参数流、安装/依赖流、global owner/根路径）已浏览器核对文字、框线、
  箭头与页脚，正文 include 正确展开。`builtin_contracts.lua` 返回 true；原 demo
  仍返回 1616。受保护 callback/排序资源协议留下一包；原章 xpcall 表述校准为失败
  返回 handler 首结果（可为非字符串或 nil），handler 自身失败才转为错误文本。
- 本地 Rust/cargo **1.96.1**：fmt、workspace all-targets Clippy `-D warnings` 通过；
  Debug/Release 各 **133 通过、0 失败、4 ignored**（109 原测试 + 24 新参考）。
  learner 三组显式初始 2/1/1 项失败；四题均完成“失败 → 临时套解通过 → 原字节恢复
  后再失败”，starter 哈希一致，恢复后刷新 mtime 避免 Cargo 缓存掩盖。默认 ignored
  与 CLI 成功退出均不表示学习者完成。
- 三个 CLI 的 **37 条关键输出**匹配；README 的 **13 条 exact 命令各执行 1 项**，
  9 正向通过、4 learner 按预期失败。正文额外的三条正向 exact 命令也各执行 1 项。
  PATH 的 `D:\lua51\lua.EXE` 确认为 **Lua 5.1.5**，实际启动 `stdlib_diff.lua`
  退出 0、输出 1550；本次不是缺 oracle 跳过。全 20 版本教学检查、mdBook 构建、
  38 页链接检查通过：**1,227 个站内链接，0 broken**。
- 本机证据独立保存在 `target/teaching-v16-2026-10-15/`：
  `full-validation/20261015T054915848303Z/{summary,metadata}.json` 记录整套命令/
  计数/源码哈希/oracle；同目录 `exact-command-sources.json` 记录逐题命令来源。
  `args/20261015T054354598325Z/`、`catalog/20261015T054534393Z/` 与
  `globals/{starter-round-trip,scoped-verification}.json` 记录局部与套解恢复；
  `final-verification.json` 记录最终站点/正文命令/交付文件哈希与基线 CI。
  这些是本机忽略产物，不随 clone 交付；永久入口是主章、三个 README、contracts 与
  examples。没有覆盖以前版本的教学证据。
- 已推送基线 `77f7576` 的 [mdBook 38687260302](https://github.com/YanqingXu/lua_rust/actions/runs/38687260302)
  成功；[Rust CI 38687260297](https://github.com/YanqingXu/lua_rust/actions/runs/38687260297)
  失败，仅 Windows Quality Gate 的 M1 foundation and quality gate 失败，三个
  Miri/ASan job 成功。此次 `gh run view --log-failed` 下载返回 HTTP 403 / API rate
  limit exceeded，未核实具体失败原因；输出见 `baseline-77f7576-failed.log`，不把
  以前提交的 Clippy 原因套用到本次。这是基线结果，不代表本批新提交的 CI。

下一教学包可继续 v0.16 的 pcall/xpcall、sort comparator 与禁止 yield 边界，围绕
回调返回/报错后如何恢复状态与保活临时对象建立反例。若选择工程整改，v0.15 已记录的
`active_varargs_gap.lua` 尾调用活动 varargs 漏根仍未修复，本轮成功不能覆盖该缺口。
后续按用户指令启动；原审计队列仍暂停在本地时间/UTC/DST。

## 2026-10-15：v0.15 独立栈、yield/resume 与 GC roots 所有权

用户要求按问题链补齐 v0.15 的反例、前后差异、状态图和验收练习，验证后更新交接。
本次完成该教学包，**没有恢复暂停的原审计整改目标**。用户随后明确要求将本批
章节、状态图、练习包和交接一并提交并推送。

- 起点为 `main@eba1321e1ce57447877242273bd11c68ab47f46a`，工作区干净；提交前 fetch
  再次确认与 origin/main 无分叉。这是上一批已推送的 v0.14 弱表/复活教学包，本批
  提交号以 `git log -1 -- book/src/versions/v15-coroutine.md` 为准，新提交 CI 须按其
  SHA 查询。本次生产 `crates/`、v0.15 快照核心与既有测试均无最终 diff，历史记录
  保持各自版本归属。
- [主章](book/src/versions/v15-coroutine.md) 按五问组织：共用 scratch 污染暂停帧 →
  每 co 保存；从入口重跑副作用 → 使用 next pc；嵌套 resume 状态/通道污染 → 保存
  并恢复 parent 与 yield 通道；Rust 仍有帧却 GC stale → thread Value 镜像；把全部
  scheduler/upvalue 索引当根 → 仅补 active upvalue 直接根。每题有预测、可运行反例、
  starter/solution include、差异、精确命令与完成条件。
- [执行包](book/code/v15-coroutine/exercises/continuation/README.md) 提供三个独立
  TODO、9 项参考和 `continuation_problem_chain` 固定 CLI。交错 A/B 的正确轨迹分别
  10→11→14、100→102→106；重跑反例把副作用 1 变 3。owner 模型用人工外层哨兵验证
  通道隔离，不冒充真实 Lua 内部状态；真实 VM 另验证交错局部值、三次 host execute
  的嵌套调用/多 yield/含 nil 多值，以及 Running/Normal 与返回方向。
- [roots 包](book/code/v15-coroutine/exercises/roots/README.md) 提供两个独立 TODO、
  9 项参考和 `coroutine_roots_problem_chain` 固定 CLI。模型只在稳定暂停点收集，
  分别展示漏镜像 stale、全 scheduler 伪根、仅全 open-upvalue 也会自举保活环；还检查
  Value 镜像不随原 Rc 槽自动更新。真实 Heap 在 Rust 槽仍持有时逐项验证八个目标，
  撤 thread root 后全部访问失败，thread `is_live==Ok(false)`，prefix 保留。真实 VM
  验证 fresh entry 跨 GC、暂停寄存器/capture/varargs 恢复计算值 46；释放时要求
  weak 的 cycle/getter/thread 三槽精确 nil、state count 0，而非只看 map 变小。
- 四幅 SVG 已在浏览器核对正文 include、列对齐、文字与箭头：独立暂停帧、恢复链、
  owner turn、执行 owner/Heap tracing 镜像分工。`resume_once.lua` 返回 true；原
  demo 仍返回 1515。临时浏览器页与预览进程已结束。
- 旧章表述已校准：A/B/C 均为 coroutine 时 C running 则 A/B 都 normal；非法 resume
  Running/Normal/Dead 保持原态，只有实际普通错误使目标 dead。Rc Vec clone 只复制
  槽别名；SuspendedFrame 没有 top 字段，恢复时由结果写回重建。Yield 不 CLOSE；实际
  detach→unwind→sync 窗口无 GC checkpoint，要求下次收集前完成镜像。旧镜像恢复中
  暂留，不能从稳定暂停模型外推全部 active/phase 转换安全。
- 本地 `rustc/cargo 1.96.1`：fmt、workspace all-targets Clippy `-D warnings` 通过；
  Debug/Release 各 **117 通过、5 ignored**（99 原测试 + 18 参考）。五个 TODO 显式
  初始分别 3/2 项失败，已完成“失败 → 临时套解通过 → 字节恢复后再失败”，原始
  starter 哈希一致。两个 CLI 的 21 条关键输出匹配；17 条逐题命令各执行 1 项：
  12 正向通过、5 learner 按预期失败。ignored 不代表练习已完成。
- 全 20 版本教学结构、mdBook 构建和 38 页链接检查通过：**1,223 个站内链接，
  0 broken**。独立证据目录 `target/teaching-v15-2026-10-15/`：
  `full-validation/20261015T035637338961Z/{summary,metadata}.json` 为整套验证与输入
  哈希；`continuation/20261015T035312541414Z/` 和 `roots/starter-round-trip.json`
  记录局部/套解恢复；`chapter-commands/` 为逐题回放；`final-verification.json` 为
  最终站点/输入哈希/基线 CI 观察。目录是本机忽略产物，不随 clone 交付；永久入口为
  章节、两个 README 与 examples，没有覆盖 v0.14 历史证据。
- **独立已知功能缺口，未修复：** [active_varargs_gap.lua](book/code/v15-coroutine/examples/active_varargs_gap.lua)
  从 Lua tail-call 向 variadic callee 传表，在 `VARARG` 搬入寄存器前 `gc_collect()`，
  实测退出 2：`line 6 (pc 7): stale GC handle for object 7`。tail-Lua 分派绕过 invoke
  临时参数根，`gc_roots` 又没有独立活动 varargs 根枚举。该项单独记为
  `known-functional-failure / functionality_pass=false`，不计入 ignored 或参考成功。
  暂停 varargs 正向验收不能证明该路径安全，也不能把默认 117 通过解释成全快照无缺口。
- 已推送基线 `eba1321` 的 CI 于 2026-10-15 12:00 CST 观察：
  [mdBook 38677872207](https://github.com/YanqingXu/lua_rust/actions/runs/38677872207)
  已成功；[Rust CI 38677872181](https://github.com/YanqingXu/lua_rust/actions/runs/38677872181)
  仍运行，GC/native Miri 与 Linux ASan 已成功，Windows quality 与另一 Miri 仍在运行。
  提交前重新查询终态：Rust CI 失败，仅 Windows Quality Gate 失败，其余三个 job
  成功。该次失败日志下载返回 HTTP 403（要求 repository admin 权限），未核实具体
  原因；输出保存于同证据目录 `push-baseline-eba1321.log`。基线结果不覆盖本批
  教学的本地验证，也不作为本批新提交的 CI 结果。

下一工程工作包可从 active_varargs_gap 精确输入处理活动 varargs/尾调用参数交接 roots，
另补正常、暂停恢复、tail-call 的对照；按后续用户指令启动。若继续教学，v0.16 可从
“宿主适配器为什么不能继续散落在 VM 内”接续。原审计队列仍暂停在本地时间/UTC/DST。

## 2026-10-15：v0.14 弱表、ephemeron 固定点与 resurrection 清理顺序

用户要求继续按问题链补齐这组反例、前后差异、状态图和练习，完成验证并交接。
本次完成该教学包，**没有恢复暂停的原审计整改目标**。用户随后明确要求将本批
章节、状态图、练习包和交接一并提交并推送。

- 起点为 `main@bdf023ec743abbf775015f9eda66d11030fc87c4`，工作区干净；提交前 fetch
  再次确认与 origin/main 无分叉。该提交是上一批已推送的短借用/回调教学包，下面保留
  其历史证据。本批提交号以 `git log -1 -- book/src/versions/v14-incremental-gc.md`
  为准，新提交的 CI 须按其 SHA 查询。
  本次生产 `crates/`、v0.14 快照核心与既有测试均无最终 diff。
- [主章](book/src/versions/v14-incremental-gc.md) 新增问题八至十：普通强扫描为何令
  弱缓存泄漏 → 按 `v/k/kv` 分类 array/map/metatable 边；单轮条件传播漏掉跨第二张
  弱键表的 value → 收集候选、drain 强边、重复到固定点；弱项过早删除无法恢复 →
  pending 图补标记先于 cleanup，Heap 返回后才执行 callback 发布 rescue root。
  每问包含预测、可运行入口、starter/solution include、最小 diff、状态图与完成条件。
- [弱表包](book/code/v14-incremental-gc/exercises/weak/README.md) 提供两个独立 TODO、
  固定 faults、共享合同、10 项默认参考验收和 `weak_problem_chain` CLI。除两表精确
  trace，还验收 1/4/16 个 productive rounds、空候选终止、撤根释放、无根 key/value
  环不能自举；真实 Heap 对照实际对象访问、弱槽精确 nil 与共享 metatable 的模式刷新。
- [复活顺序包](book/code/v14-incremental-gc/exercises/resurrection/README.md) 提供
  一个 ordering TODO、6 项默认验收和 `resurrection_problem_chain` CLI。错误版当轮
  对象仍活但四弱项已丢失；正确版按 rescue 三周期/no-rescue 两周期观察累计一次
  finalizer 与 second death。真实 Heap 通过显式 roots 模拟发布/撤销，验证 userdata、
  closure、closed upvalue 与 child/grandchild 访问，弱槽须精确 `Ok(Nil)`；真实 VM
  另分三次 execute 验证 callback 看见弱项、rescue 可用、撤根后清理，只返回 bool。
  `probe[value]=42` 验证 userdata key 真实可用，未通过重设 finalizer 改变 finalized。
- 新增三幅 SVG：模式边矩阵、ephemeron 候选轮次、Atomic 到 callback 再到 second
  death。浏览器已核对正文 include、文字/箭头和表格列对齐，临时预览进程已结束。
- 源码边界单独写明：组图/模式切换固定在 Pause/完整周期之间；周期中的弱写入可能
  保守多活一轮，模式刷新不重新扫描已标记表。Atomic 候选为空会直接退出固定点循环，
  不在入口无条件 drain gray；finalizer 补标记之后不再次运行 ephemeron。本包只验收
  弱值观察与独立弱键链，不把晚标 key/finalizer 混合图说成已正确，也不宣称官方
  Lua 5.1 全部弱表/finalizer 组合兼容。这些工程边界未在本教学包中整改。
- 本地 `rustc/cargo 1.96.1`：fmt、workspace all-targets Clippy `-D warnings` 通过；
  Debug/Release 各 **119 通过、10 ignored**（84 原测试 + 19 旧参考 + 16 新参考）。
  五组 learner 显式初始分别 3/1/3/2/1 项失败；本次三题分别完成“失败 → 临时套解
  通过 → 字节恢复后再失败”，starter 原始哈希一致。默认 ignored 不表示练习已完成。
- 四个 CLI 默认 all 的 36 条关键输出匹配；九条新增精确命令均执行到 1 项测试：
  六正向通过、三 learner 按预期失败。教学结构覆盖 20 版本，mdBook 构建与 38 页
  链接检查通过：**1,197 个站内链接、0 broken**。
- 本机证据独立保存在 `target/teaching-v14-weak-2026-10-15/`：
  `full-validation/20261015T033457998249Z/summary.json` 与 `metadata.json` 为整套
  命令/计数/工具链/源码哈希；`local/20261015T033312732289Z/` 为弱表局部与套解；
  `resurrection/starter-round-trip.json` 与 `resurrection/scoped-verification.json`
  为复活局部与套解；`chapter-commands/` 为逐条命令；`final-verification.json` 为
  最终站点/输入哈希/基线 CI 观察。此目录是忽略产物，不随 clone 交付；永久入口为
  章节和两个 README，没有覆盖旧 `teaching-v14-*` 证据。
- 基线 `bdf023e` 的 [mdBook 38675383210](https://github.com/YanqingXu/lua_rust/actions/runs/38675383210)
  已成功；[Rust CI 38675383184](https://github.com/YanqingXu/lua_rust/actions/runs/38675383184)
  已失败，仅 Windows Quality Gate 失败，其余三个 job 成功。`gh run view --log-failed`
  下载该次日志返回 HTTP 403（要求 repository admin 权限），因此未核实该次具体原因，
  不借用 `14ca9c0` 的旧 Clippy 日志当作本次证据。远端结果属于已推送基线，不覆盖
  本批教学修改的本地验证，也不作为本批新提交的 CI 结果。

下一教学包可从 v0.15 的“为什么 coroutine 不能共用一个活动栈”问题开始，继而验证
yield/resume 与 GC roots 的 owner turn；按后续用户指令启动。若用户选择工程整改，
应先对上述晚标 key/周期中模式切换另建真实故障复现，不把本轮模型当成修复证明。
原审计队列仍暂停在本地时间/UTC/DST；提交推送仍需当次指令。

## 2026-10-15：v0.14 短借用、finalizer 事件与回调重入

用户要求沿问题链继续完善短借用、事件所有权与回调重入，补齐反例、代码差异、状态图
和验收练习。本次完成该教学工作包，**没有恢复原审计整改目标**。以下旧记录属于各自
历史版本，不代表本次验证范围。

- 起点为 `main@14ca9c066af94b7a8ed2a86cf1561e6f94e7d435`，开始时工作区干净，
  本地与 origin/main 一致，提交前 fetch 再次核对无分叉。这是上一批已提交并推送的
  三问题教学包；用户随后明确要求把本次章节、状态图、练习包与交接一起提交并推送。
  本批提交号以 `git log -1 -- book/src/versions/v14-incremental-gc.md` 为准，新提交
  CI 须按其 SHA 查询。生产 `crates/`、v0.14 快照核心和既有测试均无 diff。
- [主章](book/src/versions/v14-incremental-gc.md) 接续为四个微课：长 record 借用触发
  E0499 → 提取拥有型子边；clone 队列重复消费 → `mem::take`；拥有事件不保活兄弟
  handle → 第一个 callback 前注册全批 roots；`?` 跳过事件/清理 → 保存首错、继续
  drain、`truncate(root_base)` 后返回。每题包含预测、反例、前后 include/diff、真实
  源码对应与可观察完成条件。新增三幅 SVG，并已在浏览器核对文字、箭头及实际页面。
- [短借用包](book/code/v14-incremental-gc/exercises/borrow/README.md) 用独立 rustc
  子进程检查固定 before 的 **E0499**，参考解同时验收两个 child 入队/扫描与不可达
  自环。默认参考 2 项，学习者 TODO 1 项；故意无法编译的输入不作为 Cargo target。
- [回调包](book/code/v14-incremental-gc/exercises/callbacks/README.md) 有三个独立
  hook TODO、固定故障与参考解、九项默认验收；`finalizer_problem_chain` CLI 固定
  重放 `ownership|roots|errors|real|all`，重入总额度为 1，错误版不会无限递归。
  真实 Heap 保留 owned events 后再运行下一周期，用 `userdata_is_finalized` 与
  `function_proto` 实际访问证明兄弟对象的存活；真实 VM 两次 GC 重入得到序列 123，
  第二个 callback 经 `setfinalizer(value,nil)` 校验 userdata。错误后同 VM 探针得到
  `(Type,true)`，只证明错误类型与兄弟已执行；roots 的精确清理由模型合同与源码核对。
- 教学边界已明确：NLL 结束的是 record 借用，复制 Value/handle 不自动注册 GC root；
  `finalized` 防重复发现，take 防重复取出，全批 roots 防嵌套 GC 中兄弟 second death。
  Result 首错清理不外推到 panic/abort；外层捕获的 step 不代表回调后的即时 phase。
- 本地 `rustc/cargo 1.96.1`：fmt、all-targets Clippy `-D warnings` 通过；Debug/Release
  workspace 各 **103 通过、7 ignored**（84 项原测试 + 8 项旧参考 + 11 项新参考）。
  显式启用七题初始均失败；本轮新增四题分别完成“失败 → 临时参考解通过 → 字节恢复
  后再失败”。最终 starter 哈希与初始一致，默认 ignored 不表示学习者已完成。
- 两个 CLI 的关键结果已核对。十条逐题精确命令均执行到 1 项测试：7 正向通过、3
  回调 learner 按预期失败，其中 borrow 两条为章内 aggregate 的拆分逐题命令。
  全 20 版本教学结构、mdBook 构建及 38 页链接检查通过：**1,181 站内链接、0 broken**。
- 新证据独立保存在 `target/teaching-v14-callbacks-2026-10-15/`：
  `full-validation/20261015T023801836023Z/summary.json` 与 `metadata.json` 记录整套
  命令/计数/源码哈希；`borrow-validation/20261015T023334878-summary.json` 和
  `starter-round-trip.json` 记录套解与恢复；`chapter-command-checks/` 记录逐题命令；
  `final-verification.json` 记录站点检查、最终输入哈希和 CI 观察。本目录为本机忽略
  产物，不随 clone 提供，永久入口是章节与两个 README。没有覆盖上一批的
  `target/teaching-v14-2026-10-15/` 历史证据。
- `14ca9c0` 的 CI 与本次未提交修改分别记录。2026-10-15 10:40 CST 观察：
  [mdBook 38671128155](https://github.com/YanqingXu/lua_rust/actions/runs/38671128155)
  已成功；[Rust CI 38671128205](https://github.com/YanqingXu/lua_rust/actions/runs/38671128205)
  仍运行中，GC/native Miri 与 Linux ASan 已成功，Windows quality 和另一 Miri 仍运行。
  提交前重新查询终态：Rust CI 失败，仅 Windows quality 失败，其余三个 job 成功；
  失败日志显示 Clippy 的 `fetch_update` deprecated 导致 3 个编译错误，Debug/Release、
  doctest、cargo doc、audit 均通过。日志存于同证据目录的
  `push-preflight-ci-14ca9c0.log`；该结果属于 `14ca9c0`，不代表本次尚待推送提交的 CI。

下一教学包可单独从 weak key 的“只传播一轮为什么不够”构造 ephemeron 固定点反例，
再处理模式变更与 resurrection 的清理顺序。按后续用户指令启动；原审计队列仍暂停在
本地时间/UTC/DST。提交与推送由当次指令授权，不从上一次推送请求自动延续。

## 2026-10-15：v0.14 强引用增量 GC 问题链

用户要求围绕“分步回收 → 跨调用保存进度 → write barrier”完善 v0.14，补齐可运行
反例、前后代码差异、状态图和验收练习，验证后更新交接。此次只推进该教学工作包，
**没有恢复原主线审计整改目标，也没有扩展 weak/finalizer 教学工作包**。

- 起点为 `main@fdb77ad517a91e9d5b1762b7d5a9a40f45efae6c`，工作区干净。用户随后明确
  要求将本批章节、状态图、练习包和交接一并提交并推送；提交号以
  `git log -1 -- book/src/versions/v14-incremental-gc.md` 为准，新提交的 CI 须按其 SHA
  单独查询。生产 `crates/`、v0.14 快照核心 GC 和既有测试均没有最终 diff。
- [v0.14 章节](book/src/versions/v14-incremental-gc.md) 按三个反例组织：一次做完导致
  没有中间返回点；每次重启导致预算 1 不前进；漏 barrier 导致新接入的 child 被回收。
  每题附可编译 starter/reference include、最小 diff、精确命令与完成条件。
- [实验包](book/code/v14-incremental-gc/exercises/README.md) 的独立强引用小模型提供
  错误/修复行为，`gc_problem_chain` example 固定重放；模型不是历史 VM 快照，
  不支持周期中分配、weak/finalizer 或完整 Lua 对象合同。真实 Heap 另验证同一四对象
  八次轨迹、full/step 统计及实际存活分类，以及预分配 child 的 table_set 保活。
- 两幅 SVG 解释持久队列/游标和 late-edge barrier。已核对实际网页显示、图中文字和箭头。
  Mark/Atomic 重扫 roots、Atomic 末建立 sweep snapshot、marked bool 与 gray Vec 的
  区别均已校准；budget 是工作计数，不能外推恒定暂停时间。
- 本地 `rustc/cargo 1.96.1`：v0.14 fmt、all-targets Clippy `-D warnings` 通过；
  Debug/Release workspace 各 **92 通过、3 ignored**（84 项原测试 + 8 项参考验收）。
  显式启用三题初始全部失败；临时套用参考解后三题通过，恢复 starter 后再次验证三题
  失败。默认 ignored 不表示学习者已完成，也不是隐藏未修复的原有回归。
- 四条章节精确命令已回放：三条正向各执行到 1 项通过，一条学习者单题按预期失败。
  CLI 的文中关键输出已核对。全 20 版本教学结构、mdBook 构建、38 页链接检查通过：
  1,159 个站内链接，0 broken。本机日志、工具链、基线 SHA 与输入哈希保存到
  `target/teaching-v14-2026-10-15/final-verification.json` 及同目录日志；该目录为忽略
  产物，不随 clone 提供，永久复现入口为章节和实验 README。
- 上一个已推送教学提交 `fdb77ad` 的远程终态已重新核对：
  [mdBook 38666400522](https://github.com/YanqingXu/lua_rust/actions/runs/38666400522) 成功；
  [Rust CI 38666400608](https://github.com/YanqingXu/lua_rust/actions/runs/38666400608) 失败。
  Windows 官方 required lane 的 Rust `constructs.lua` 约 30 秒超时（其余 9 项与双
  oracle 各 10 项通过）；M1 Clippy 在远端 Rust 1.99.0 因生产代码的
  `AtomicU64::fetch_update` deprecated 失败，位置是 `state_handle.rs:134`、
  `gc/object_id.rs:38`、`heap.rs:26`。其余三个 job 成功；该提交没有修改生产 crates，
  不把日志写成 v0.13 教学回归，也不把超时称作已修复或确定的 flaky。

下一教学工作包可从“借用对象时还要修改工作队列”的最小 Rust 反例继续，再讲 finalizer
事件所有权与重入；weak/ephemeron 固定点单独安排。后续按新用户指令启动。原审计
队列仍从本地时间/UTC/DST 接续，历史验证与本次教学验证保持各自版本归属。

## 2026-10-15：v0.13 GC 教学工作包

本次用户要求参考《用 Rust 实现 Lua 解释器》，优先完善 v0.13 的小步迁移、对象状态图
和可执行练习。此次只推进该教学工作包，**没有恢复下面暂停的主线审计整改目标**。

- 开始时 `main@14718c8ce3364f965b9551248b87971e2c4ed852`，工作区干净，与原交接一致。
  生产 `crates/` 和教学快照的 GC 核心行为均未修改。用户随后明确要求将章节、状态图、
  练习包和本交接一并提交并推送；本批提交号以 `git log -1 -- book/src/versions/v13-runtime-gc.md`
  为准，远端状态以对应提交的 GitHub Actions 运行记录为准。
- [v0.13 章节](book/src/versions/v13-runtime-gc.md) 新增六个对照微课、每步定向验收、
  双表环与 CLOSE 两幅 SVG；明确实际 owner 是 `Vm.heap`、Proto 不属于 arena，以及
  roots/edges、mark 失败撤销和最终 Drop 的边界。微课对照 v0.12/最终 v0.13，
  不宣称仓库新增了六份可运行中间快照。
- [可执行练习包](book/code/v13-runtime-gc/exercises/README.md) 含四项 TODO starter、
  共享行为合同及可选参考解：双表环、宿主 registry 租约、CLOSE 修复、primitive roots。
  另验收部分 mark 失败后重试，以及真实 VM 的 primitive metatable 保活。
- 本次验证限定到 v0.13 与教学站点：fmt、all-targets Clippy 通过；Debug/Release
  workspace 各 **78 通过、4 ignored**。ignored 为学习者 TODO，显式启用时初始四项失败；
  临时套用参考解后四项通过，随后恢复 starter。不能将默认通过视为读者已完成练习。
- 真实 primitive root 故障注入得到 `Metamethod / stale GC handle`；恢复原行后同一定向
  测试通过，核心源码没有最终 diff。两幅 SVG 已在浏览器检查文字与箭头。
- 章节 13 条精确测试命令分别执行到 1 项通过。全 20 版本教学结构检查、mdBook 构建、
  38 页链接检查通过（1,141 站内链接，0 broken）。本机命令日志与源文件 SHA-256 在
  `target/teaching-v13-2026-10-15/chapter-command-results.json` 及同目录日志；该目录
  是忽略产物，不随 clone 提供。永久复现入口是上面的章节命令与练习包说明。
- 重新查询远程 CI：代码检查点 `834eee3` 的 [Rust CI 34686860957](https://github.com/YanqingXu/lua_rust/actions/runs/34686860957)
  最终失败，日志中 Rust `constructs.lua` 超时；其 mdBook 成功。
  后续交接提交 `14718c8` 的 [Rust CI 34687179499](https://github.com/YanqingXu/lua_rust/actions/runs/34687179499)
  四项成功。保留两次运行各自的结果，不能抹掉历史超时；旧运行不覆盖本次教学工作包，
  本批新提交须按自己的 SHA 查询 CI。

下一教学任务可沿同一模板完善 v0.14，按新的用户指令启动；原主线队列仍从本地时间/
UTC/DST 修复接续。提交、推送与线上发布均需按当次用户指令处理。

## 原审计目标和暂停位置（2026-09-12）

- 用户最初要求：拉取最新代码，深度审计项目，给出方向；随后要求“按照建议依次推进！”。
- 本线程目标工具显示 objective 为“按照建议依次推进！”，状态为 **paused**；目标未完成。
- 用户要求生成交接供下次会话识别，随后明确要求提交并推送。本次仅交付文档，没有恢复整改或启动新修复。
- 用户要求快速收敛，避免扩大当前工作包。下次用户要求继续时，直接从下面的首项接续。
- 下一任务：**修复 `os.date` / `os.time` 把本地时间按 UTC 处理的问题**，再处理受限大负载与剩余兼容性。
- 完整 M1/M2、资源预算和 M3 都未完成；完整 M0 或 M1 基础门通过不表示总目标完成。

## 已交付检查点

代码仓库：`G:\github\lua_rust`，PowerShell，分支 `main`。
远端：`https://github.com/YanqingXu/lua_rust.git`。

最近已推送的代码修复检查点：
[`834eee3d9f698eb7ea997d1434c5c9493247f12e`](https://github.com/YanqingXu/lua_rust/commit/834eee3d9f698eb7ea997d1434c5c9493247f12e)
`fix: propagate nested compile errors and align Windows popen`。
推送后已核对远端 main 与本地 HEAD 完全一致，生成本交接前工作区干净。
用户已要求将本交接单独提交并推送；文档提交号以 `git log -1 -- SESSION_HANDOFF.md` 为准。
本次只涉及 AGENTS.md、SESSION_HANDOFF.md、plan.md 和整改状态入口，源码检查点仍为上述提交。

| 已推送批次 | 内容 |
| --- | --- |
| `4024f9e` | Runtime/GC 公开访问、异常边界、普通 Lua 调用迭代调度及审计基础整改 |
| `33aba8e` | debug hook/协程 traceback、原始诊断字节、REPL 续行恢复、无循环 break 拒绝、bytecode 工具诊断 |
| `834eee3` | 嵌套 codegen 错误传播、失败子 Proto 拒绝发布、Windows 精确 `ls` 可用性检查、对应回归及文档 |

交接时远程状态（2026-09-12 17:54 CST 的观察，后续须重新查询）：

- [Rust CI 34686860957](https://github.com/YanqingXu/lua_rust/actions/runs/34686860957)：运行中，不能写成通过。
- [mdBook 34686860956](https://github.com/YanqingXu/lua_rust/actions/runs/34686860956)：成功。
- `4024f9e` 和 `33aba8e` 的 Rust CI 四项及 mdBook 均成功；这些是历史结果。
- 本地构建、门禁、探针和代理工作均已结束，没有可继续轮询的本地进程句柄。

## 下次会话先做什么

1. 读取本文件，执行 `git status --short --branch` 和 `git log -3 --oneline`，核对是否已有后来修改。
   如果用户要求同步远端，先 fetch；存在未提交内容时先保护，不使用 reset/clean 覆盖。
2. 查询代码检查点 `834eee3` 的 CI 终态及其后文档提交的检查状态。如果失败，先读取失败 job
   日志并处理与对应批次有关的问题。
3. 读取下述时间转换反例和源码，再冻结实际输入、建立最小回归。不要重新做一次全项目审计。
4. 按独立工作包修复和验收，更新交接。提交/推送按当次用户指令处理，不把上一次已完成的推送
   请求扩大为以后每个工作包自动推送。

```powershell
gh run view 34686860957 --json headSha,status,conclusion,url,jobs
gh run view 34686860956 --json headSha,status,conclusion,url
gh run list --branch main --limit 5 --json headSha,workflowName,status,conclusion,url
```

## 首项：本地时间、UTC 和 DST

这是已确认的问题，尚未修改 `crates/lua_stdlib/src/os.rs`：

- `lua_os_time` 从约第 63 行开始，以 civil-date 公式计算 UTC，忽略本地时区及 isdst。
- `lua_os_date` 从约第 212 行开始，去掉 `!` 后仍在第 243 行无条件调用
  `DateParts::from_timestamp`，第 268 行把 table 的 isdst 固定为 false。
- 固定 C++ 的 `src/lib/oslib.cpp` 第 382 行使用 mktime，第 418/421 行分别使用
  gmtime_s/localtime_s；不能仅改显示小时而不改反向转换、日期跨界和 DST。

最小 Lua 反例（即使本机 target 证据丢失也可重建）：

```lua
local t = 1577836800
print(os.date('%Y-%m-%d %H:%M:%S', t), os.date('!%Y-%m-%d %H:%M:%S', t))
local l, u = os.date('*t', t), os.date('!*t', t)
print(l.hour, u.hour, l.isdst, u.isdst)
print(os.time{year=2020, month=1, day=1, hour=8, min=0, sec=0})
print('continued')
```

每个引擎使用独立子进程，分别设置其私有环境 `TZ=UTC0`、`TZ=CST-8`，不能改宿主全局 TZ。
已运行两时区 × Rust/固定 C++/stock 共六个进程，全部正常退出、空 stderr、输出 continued。

| 时区 | 固定 C++ 与 stock | 旧 Rust |
| --- | --- | --- |
| UTC0 | 本地/UTC 均为 2020-01-01 00:00:00；反向 os.time=1577865600 | 相同 |
| CST-8 | 本地 08:00:00、UTC 00:00:00；table hour=8/0；反向 os.time=1577836800 | 本地仍为 00:00:00、hour=0/0；反向仍为 1577865600 |

原始证据：`target/os-time-boundary-evidence/from-files-probe/observations.json` 与 `probe.lua`。
这里的 Rust 观测使用嵌套 codegen 修复后的旧快照 `0def4d314fd8…`，**不是最新 app**；
被观察的 os.rs SHA 为 `a96f6e3312a84c79b8271c54c8e311d1c8cce62d74bb46715781b03192492f10`，
后续 popen 批次没有修改该文件。新会话需在实际新输入上复现，不能把历史二进制冒充 HEAD。

建议验收边界：

- 先核实两个 oracle 对本地/UTC、`*t`/`!*t`、省略字段、isdst 和非法范围的实际合同；
  DST 尚未做完整矩阵，不把固定 UTC+8 反例外推为 DST 已验证。
- 回归覆盖日期跨界、反向转换及错误后继续执行；保留原有非法字段、范围和溢出的可捕获错误。
  相关既有回归：`crates/lua_app/tests/stdlib_error_boundary.rs`、
  `tests/characterization/os-time-error-boundary.lua`。
- 若调用平台时间 API，先确认有效范围和错误路径，避免重新引入 CRT 参数失败 abort 或 native panic。
  每个时区独立进程，避免进程级时区缓存影响测试。
- Debug/Release 定向双 oracle；随后运行与变更相关的门禁。若引入 unsafe/FFI，按真实边界补验证；
  不为不支持的宿主 OS 调用伪造 Miri 通过。

## 后续队列和未关闭项

| 顺序 | 工作包 | 已知事实与完成条件 |
| --- | --- | --- |
| 1 | 代码检查点与交接提交的远程 CI 收尾 | 查询实际终态，有失败先定位；不重复启动已经结束的本地任务 |
| 2 | 本地时间/UTC/DST | 上述已证实反例；以双 oracle 与 Debug/Release 回归闭环 |
| 3 | 大负载、资源与其余平台/native diagnostic | gc/sort/verybig 曾在并发负载下超时，尚不能认定挂死；隔离并限制预算观察后再决定修复 |
| 4 | M3 首批 reader/verifier | 两种 chunk 格式显式识别、受限解码、字节码验证、失败不发布部分图；再接 serializer 和 Lua 入口 |
| 5 | C ABI / native module | 在调用、异常和生命周期合同稳定后推进；当前未完成 |

- 受限大负载：原版 gc 约 200k 节点、sort 30k、verybig 70k；此前 60 秒超时受并发影响。
  `big.lua` 的 >4 GiB 压力、TestC 私有测试和完整 `all.lua` 不在已验证范围。
- main 已越过交互段，仍在第 135 行 string.dump 未实现失败；calls/main 的 binary chunk 依赖
  归 M3，不通过放宽断言或仅延长超时关闭。
- Windows files.lua：最新 Rust/C++ 八次正常完成；stock 四次在第 59 行 rename 失败。
  四组 runner 正确保留 exit 1；files 不在要求三引擎全部通过的严格 CI 子集。
- popen：空 COMSPEC 下 C++ abort、Rust 受控错误；NUL 前缀、无效 COMSPEC 通用 pipe 路径、
  旧模式错误的位置及普通 shell 本地化差异仍明确未对齐，不算 parity 通过。
- 9 个历史 non-official debt 是 2 个 userdata 显示和 7 个内存计量差异。
  callback/xpcall/tostring 的双 oracle 冲突仍需记录兼容目标，不能扩大 normalization 隐藏。
- 自动 GC pause、进程 RSS、内存配额、可恢复 OOM 和任意宿主栈安全没有完整证明。
  现有普通 Lua 活动帧上限为 512，相关通过结果不能外推全部 native 重入。
- [M3 合同设计](docs/rust_migration/m3_contract_design.md)：固定 C++ dump 与 stock payload 不同；
  首批先做 reader/verifier 和原子发布，再做 serializer，最后公开 C ABI。

## 验证证据索引：不要混用源码版本

| 范围 | 结果 | 本机证据 |
| --- | --- | --- |
| 最新 popen 最终输入 | 377 文件，清单 ed0cc750243c22cc…，验收后零漂移 | `target/files-popen-evidence/source-final.json`、`final-verification.json` |
| 最新完整 M0 | 75,542 ms，0 hard failure，83 match / 9 debt，无 skip/smoke | `target/files-popen-evidence/m0-final.json` |
| 最新 popen 定向 | Debug/Release 各 8/8；各九项 raw 回放及三个独立边界探针完成 | 同目录 `current-work/report.md`、`independent-boundaries/final-review.md` |
| 最新静态检查 | fmt；lua_app/lua_stdlib all-targets Clippy -D warnings 通过 | 同目录 `clippy-final.log`、`final-verification.json` |
| 最新 files 原版 | 12 进程：Rust/C++ 8 完成，stock 4 失败，无超时 | 同目录 `official-final-*.json`、对应 `.validation.json` |
| 前一个嵌套 codegen 快照 | 完整 M0/M1 基础门，1,080 普通测试，默认 Miri 2/2；Debug/Release 各十个严格官方文件与工具合同，共 218 进程核验 | `target/compiler-error-propagation-evidence/final-verification.json`、`m1-final.json`、`miri-compiler-boundary.log`、`final-independent-validation.json` |

最新 popen 后未重跑完整 M1/Miri；按用户快速收敛要求结束本地验证。
不能将前一个快照的 1,080 测试计数或 Miri 结果写成最新整个提交的完整验证。
嵌套 codegen 的清单为 `b29389c486d4537440e68e10e655a975179d6a2c9453f695f3f079a94023f25a`。

最新 app SHA-256：

- Debug：`a2b149894295c125696d5763df8358b34f91b060fb788b7f146269f5e215c8b9`
- Release：`e40fb59e53345083d0505e0c2a3a83bb0c8417afdf0f223da5bc40037f1ca797`

这些 target 目录是本机忽略的产物，**不会随 git clone 到另一台机器**。缺失时使用提交内的
整改报告、永久测试和本文件中的反例重建，并为新结果记录新身份，不声称旧原始证据仍在。
摘要入口：[嵌套 codegen 整改](docs/audits/2026-09-12-nested-codegen-remediation.md)、
[popen 整改](docs/audits/2026-09-12-popen-remediation.md)、
[官方源码 lane](docs/audits/official-source-lane.md)、
[Runtime 基线](docs/audits/runtime-performance-baseline.md)。

## 工具和 oracle 提醒

- Windows Python 使用 `py -3`；本机 `python`/`python3` 指向 WindowsApps 别名，曾返回 9009。
- `.cargo/config.toml` 默认 target 为 x86_64-pc-windows-msvc，app 在
  `target/x86_64-pc-windows-msvc/{debug,release}/lua_app.exe`。
- C++ pin：`87c15e69ceb94eb74e28226ccbefb7e196635711`，本地源在
  `target/oracles/audit-cpp-source`；不可在普通修复中移动 pin。
- C++ app：`target/oracles/lua_cpp/build/Release/lua_app.exe`，SHA
  `d495f6710ac5063a7061446b24f66a00ca8dbbe1d8841875048e13bd3e89c329`。
- stock：`target/oracles/lua-5.1.5/build/Release/lua51.exe`，SHA
  `47fd81e426c4f598e9fe2fdf28a0b36df721984ecfd66c8991eeef763693978d`。
- 官方原文件使用 `tools/run_official_suite.py`，不是不存在的 run_official_source_lane.ps1。
  保持原 Lua 文件、完成标记和失败退出码；不要给 stock 参数偷偷传 C++ 来取得全绿。
- 全门禁入口：`tools/m0_compatibility_gate.ps1`、`tools/m1_foundation_gate.ps1`。
  需要固定 `-CppRoot target/oracles/audit-cpp-source` 和新的 ResultPath；M0 另指定 ResultDirectory。
  full 验收不加 skip/smoke。先检查 cargo-audit 可用性；本机额外工具曾位于
  `target/audit-2026-09-12/audit-tool/cargo-audit-x86_64-pc-windows-msvc-v0.22.2`。
- 不覆盖旧门禁 JSON 或失败历史。输入冻结工具为 `tools/run_runtime_baseline.py` 的
  snapshot_source，扩入 tests/tools/.github 后记录全部输入。工作区构建不等于从 ZIP 重建。
