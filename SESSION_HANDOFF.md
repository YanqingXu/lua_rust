# 未完成目标交接

更新：2026-10-15，Asia/Shanghai。此文件是跨会话接续入口；详细整改状态见
[审计整改](docs/audits/2026-09-12-remediation.md)，长期计划见 [plan.md](plan.md)。

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
