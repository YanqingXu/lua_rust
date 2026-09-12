# 2026-09-12 审计整改与后续工作

状态：进行中。对应用户指令“按照建议依次推进”。原始审计保留在
[2026-09-12.md](2026-09-12.md)，不将修复后的现象改写成原始基线。

跨会话接续见 [未完成目标交接](../../SESSION_HANDOFF.md)。嵌套 codegen 与 popen 批次
已提交并推送为 `834eee3d9f698eb7ea997d1434c5c9493247f12e`；原目标目前暂停，
下一项为本地时间/UTC 转换。新提交 CI 的动态状态在交接中记录，继续前须重新查询。

## 基线与边界

- 开始时已执行 `git pull --ff-only`，结果 Already up to date；HEAD 和
  origin/main 均为 `503bf99d12a5ebae81862ad3ea4dbf7e97af38ca`。
- 第一批整改已提交并推送为 `4024f9ea706654f20cb8e63686ac1cf5614ffb39`。
  该提交的 [Rust CI](https://github.com/YanqingXu/lua_rust/actions/runs/34677728567)
  四个 job（Windows quality、Miri、GC Miri、Linux ASan）全部成功；
  [mdBook](https://github.com/YanqingXu/lua_rust/actions/runs/34677728565)
  构建与部署成功。原始状态归档在 `target/push-4024f9e-ci`。
  下文原有本地验证对应该批整改；后续工作区变化另列证据，不继承该提交的远程 CI 结论。
- debug、诊断字节、REPL/break/bytecode 后续批次已按用户指令提交推送为
  `33aba8e5c18072fc8d2711d3c827b30a91e3a0ce`，远端 main 已核对一致。该提交的
  [mdBook](https://github.com/YanqingXu/lua_rust/actions/runs/34684962335) 已成功；
  [Rust CI](https://github.com/YanqingXu/lua_rust/actions/runs/34684962347) 的四个 job
  （Windows quality、Miri、GC Miri、Linux ASan）均已成功。
  原始状态保存在 `target/push-repl-break-bytecode-20260912/`。
  下述嵌套 codegen 与 Windows popen 修复纳入本次提交，不继承该提交的远程结果。
- 固定 C++ oracle 仍为 `87c15e69ceb94eb74e28226ccbefb7e196635711`；
  第二 oracle 为官方 Lua 5.1.5。没有移动 oracle pin。
- 本机使用 Windows x86_64、stable Rust 1.96.1；默认 Miri 使用
  `nightly-2026-07-29`，没有关闭 Stacked Borrows 检查。

## 工作包状态

| 顺序 | 范围 | 当前状态与剩余验收 |
| --- | --- | --- |
| 1 | A01/A02：Runtime 和 GC 公开访问边界 | 公开访问修复、真实源码/callback 和 GC/栈窗口定向默认 Miri 已通过；最终完整 M0/M1 基础门通过，不外推全部 unsafe 安全 |
| 2 | A03/A04/A09：Lua 异常与回归硬门 | rawset/os.time 子进程 Debug/Release 通过；窄签名策略、fixture 修复、doctest 与新 Miri 入口已实现；普通 Lua 调用已改为迭代调度，Debug/Release 及 1/2/8 MiB 宿主线程栈通过；本轮完整 M0/M1 基础门通过 |
| 3 | A05–A08：表、浮点、参数/error object | 表/浮点/参数和共享数字 formatter 定向双 oracle、Debug/Release 通过；普通错误来源修复已关闭 metamethod-arith 具体 debt |
| 4 | 余下 M2、状态文档、workload 基线 | db/errors、REPL、裸 break、bytecode 诊断及嵌套 codegen 错误传播均已分批验收；main 仍有 dump 依赖。Windows files/popen 可用性已定向验收；下一项为本地时间/UTC 转换错误，随后大负载；历史性能基线及 oracle 冲突独立保留 |
| 5 | M3 binary chunk / C API / native module | 已形成基于双格式源码与 dump 实测的合同设计；实现保持待前置风险收口，不发布未稳定 ABI |

## 已落地的关键改动

### 嵌套 codegen 错误传播（本次提交）

完整说明见 [嵌套编译错误整改](2026-09-12-nested-codegen-remediation.md)。七处被忽略的
Result 已显式向上传递，公共编译出口保留首错，失败子函数恢复外层结构后拒绝附加
Proto，repeat 临时状态在错误路径恢复。没有改变合法路径 opcode 顺序，也未修改 oracle。

- 完整 M0：263,843 ms，0 hard failure，83 match / 9 debt。
- 完整 M1 基础门：284,275 ms，0 hard failure；fmt、Clippy、Debug/Release 全 targets、
  doctests、rustdoc、audit 均通过，无跳过；普通测试 1,080 项，无 ignored。
- 默认 Miri：publication 失败回收与真实 reader GC/后续闭包/Runtime close 两项通过，
  382.76 秒，未设置 `MIRIFLAGS`。
- Debug/Release 各十个官方严格文件 × 三引擎通过；各自 CLI 18/18、REPL 8/8、
  bytecode 合同 15/15 通过，218 次执行的原始输出、退出码和完成标记已独立核验。
  保留 Rust literals 的上游 locale 条件跳过。本轮未重跑完整 19 项或原版 main。
- Cargo/配置/源码/fixtures/tools/CI 共 376 输入的最终清单 SHA-256 为
  `b29389c486d4537440e68e10e655a975179d6a2c9453f695f3f079a94023f25a`；
  验收前后无漂移。最终二进制与独立快照一致，七条 M1 步骤引用对应的六份产物及索引已另存。

证据集中在 `target/compiler-error-propagation-evidence/`，主入口为 `final-verification.json`、
`m0-final.json`、`m1-final.json` 与 `final-independent-validation.json`。Miri 启动后只增加
了编译器 integration tests，实际参与 Miri 的生产库和 boundary target 与最终快照一致。
本轮纳入当前提交。父状态恢复不是所有字段事务回滚，双方共同跳过多余 local 初始化的
行为未改变；不能外推为拒绝所有含非法文本的源码。M1/M2 整体仍未完成。

### Windows popen 可用性（本次提交）

已补齐固定 C++ 对精确命令 `ls` 的创建管道前检查，错误包含 Lua 调用位置。
Debug/Release 新增测试各 8/8，通过后最终三个独立边界探针完成，未改 oracle 或
普通 pipe close 合同。完整说明及原版 files 结果见 [popen 整改](2026-09-12-popen-remediation.md)。
下一项是已用固定时间戳、双 oracle 复现的本地时间/UTC 转换错误。

### REPL、break 与 bytecode 联合批次（已提交 33aba8e）

本轮完成 [REPL 续行和错误阶段](2026-09-12-repl-continuation-remediation.md)、
[无循环 break 挂死及加载器来源](2026-09-12-break-scope-remediation.md)、
[bytecode 工具诊断](2026-09-12-bytecode-diagnostic-remediation.md)。后者保留 FF，
并仅在工具异常出口按固定 C++ 的 C 字符串合同处理 NUL。

- 最终完整 M0：379,238 ms，`fullGatePassed=true`，0 hard failure，83 match / 9 debt。
- 最终完整 M1：540,280 ms，`foundationPassed=true`，0 hard failure；fmt、Clippy、
  Debug/Release 全 targets、doctests、rustdoc、audit 均通过，无跳过。普通测试清单
  1,056 项，无 ignored；新增 REPL 19、compiler break 7、bytecode 诊断 3 项。
- 最终 Debug 官方严格子集十文件 × 三引擎完成；Release db/errors × 三引擎完成。
- Debug/Release 各自 CLI 18/18、REPL 8/8、bytecode 工具 15/15 通过；工具合同
  含 12 项跨语言比较与 3 项 Rust 扩展。合法四类循环的 break、捕获变量及随后 GC
  在 Debug/Release/固定 C++ 三个进程中执行通过，原始输出一致。
- 原版 main 在最终 Debug/Release 都越过交互段，停在第 135 行 `string.dump`
  未实现；文件仍失败，不加入严格通过子集。固定 C++ 完成；stock Windows 仍在
  临时模块 require 阶段失败。没有重跑完整 19 项或改写历史 11/19。

完整产物在 `target/repl-break-bytecode-evidence/`：`m0-final.json`、`m1-final.json`、
`test-list.log`、`official-*-final.json` 和各工具 `report.json`。M1 步骤 JSON 另存
`m1-step-artifacts/`，最终四个二进制 SHA 在 `binaries-final.json`。
Cargo/配置/源码/fixtures/tools/CI 共 372 个输入文件归档为 `source-final.zip`，
清单 SHA-256 `d9423c05874c8c5e72898cec49376517d7f60efb8523010895abb3ad9e844292`，
验收结束逐文件核对无变化。由同一工作区构建，未声称从 ZIP 重建。

本批未新增 Miri 运行，前批定向 Miri 证据按各自源码身份保留。`33aba8e` 收录 debug、
诊断字节及上述联合批次，提交前已核对 372 个输入文件与验收快照一致。远程 CI
须按该提交 SHA 单独验收，不能继承 `4024f9e` 的结果。该联合门没有覆盖随后发现的
嵌套 emitter 吞直接错误 P1：`do 1=2 end` / `if true then 1=2 end` 在该快照错误地编译成功。
该项原始证据见 break 报告末节，后续修复与最终验收见上节。

### 诊断字节后续批次（前一已验证工作区快照）

完整说明见 [诊断字节整改](2026-09-12-diagnostic-bytes-remediation.md)。Parser、
codegen、VM、加载器与 CLI 使用 owned 原始字节传递诊断，保留源码/token/field
名称中的高位字节；Lua 显式错误对象仍保留类型与 identity。默认层级的数字 error
按两 oracle 的共同合同添加位置并转为 string。

- 最终完整 M0：74,375 ms，0 hard failure，83 match / 9 debt。
- 最终完整 M1：50,465 ms，`foundationPassed=true`，0 hard failure；fmt、Clippy、
  Debug/Release、doctests、rustdoc、audit 均通过。普通测试清单 1,027 项，无 ignored。
- 默认 Miri：两项真实加载器/错误发布及 Runtime close/drop 后读取诊断回归通过，
  282.20 秒，未设置 `MIRIFLAGS`。
- 最终 Debug 官方严格子集：十文件 × 三引擎通过；Release 的 db/errors × 三引擎通过。
- 产物在 `target/errors-byte-evidence/`，最终门为 `m0-final.json`、`m1-final.json`；
  最终源码清单 369 文件，SHA-256
  `091db23ba5ea1de4c426d5ccb35cbada21574ae195a0d8379c466ebc5e6d15b4`。

首次 M1 的两处 Clippy 失败已修正并保留失败记录。该验收快照形成时 debug 与诊断
字节批次尚未提交；现随上述联合批次收录。require 与 reader chunk name 的 oracle
分歧仍单独保留，原版 errors 完成不意味着所有诊断正文已完全对齐。

### debug 后续批次（前一已验证工作区快照）

完整说明见 [调试库整改](2026-09-12-debug-hook-remediation.md)。本批将 line/PC
历史移到调用帧，移除 traceback 人为重复帧，并修复 debug 帧访问过早持有可变 GC
所触发的默认 Miri UB、自身协程层级偏移、跨协程 name-only 结果发布到错误栈。
删除无用途的 `DEBUG_PROTO` 后，根库存为 23 类，真实活动 Proto 根仍有回收测试。

- 最终完整 M0：191,163 ms，`fullGatePassed=true`，0 hard failure，83 match / 9 debt。
- 最终完整 M1：209,515 ms，`foundationPassed=true`，0 hard failure，Debug/Release
  全 targets 通过；普通测试清单 1,013 项，无 ignored，doctest 另行通过。
- 默认 Miri：debug 访问与 hook 生命周期两项通过，163.54 / 192.75 秒，未禁用别名检查。
- 最终 Debug 官方严格子集：9 文件 × 3 引擎通过；Release 的 `db.lua` 三引擎通过。
- 产物统一在 `target/debug-hook-evidence/`，完整门为 `m0-verified.json`、
  `m1-verified.json`；源码/fixtures/tools/CI 共 365 个文件的清单 SHA-256 为
  `f87ec0ad8591e6a72dea86da80af6d506cc65f27164635eb5a9da901cf5e8099`。

本批验收时为工作区修改，现随上述联合批次收录。后文原有
1,009 测试、24 类根库存与性能数字保留为前批快照，不作为本批当前数量。

### 第一批整改

`RuntimePartsMut::split_mut` 只提供显式组件借用。需要原生服务时，通过
`with_context` 只借用 LuaState，或通过 `with_context_gc` 显式借用 GC、动态借用
StringPool。后者拒绝 LuaState 动态恢复同一 GC。CLI、REPL 结果消费与点路径补全、标准库初始化
与资源关闭调用链已迁移；资源关闭和补全的字段读取均改为显式 collector 与 StringPool。
补全迁移由完整 M0 拦下漏项后修正，原 Debug/Release 8-case 合同通过，见
[REPL 补全修复](2026-09-12-repl-completion-remediation.md)。

`GarbageCollector::with_mut` 与 publication 的对应入口现在提供 `GcWrite`。
它只暴露具体 payload mutator 和只读 Deref，不暴露 `&mut T`、整对象替换、可变
header 或提前析构。Header 的链接、标记与颜色修改限制在 core crate 内。
写回调在正常返回和 panic 展开时都执行容量对账和写屏障。固定对象在后续周期
也会追踪其子对象。

进一步检查发现同步 sweep 能释放增量 sweep 的前驱游标。同步 mark/sweep 现在
撤销已有增量进度，各增量阶段在 Debug/Release 都验证调用顺序。身份查找保留
原始 NonNull 指针，不再从整数地址重建指针。

Typed userdata 记录具体 Rust `TypeId`，提供检查后的 `typed_ref/typed_mut`；
析构之前清除类型身份。IO 按真实构造类型读取，不再把相同大小、相同元表当作
`IoFileData` 的证明。原始字节视图在 typed payload 存活时仍不可用。

实际源码 Miri 路径另外发现 Parser recursion guard、LuaState native-request guard
和 Runtime activation-session guard 的裸指针被后续可变借用使之失效。Parser
改为独立拥有的深度计数，后两者改为直接持有并转借可变引用。

其他工作包的实现细节：

- 表删除保留不追踪、不解引用的身份墓碑，同键重插复用槽位；不同键 churn 有界压缩。
  `try_next` 区分删除后的合法游标和无效游标。±0 共用一个键并保留首次插入符号。
- DIV/MOD 采用 Lua 浮点结果；Lua `==` 和 `rawequal` 使用 IEEE 数值相等。
  编译器的常量身份比较保持独立，未把 ±0 常量合并。
- [rawset/os.time 整改](2026-09-12-stdlib-remediation.md)。
- [字符串参数与 error object 整改](2026-09-12-string-remediation.md)。
- [共享数字 formatter](2026-09-12-number-format-remediation.md)：1,076 数值 × 16 格式对两个 oracle 无差异。
- [错误来源整改](2026-09-12-error-provenance-remediation.md)：结构化来源与显式 error object 分离；普通 pcall/Main 定位，保留既有 resume 策略；移除字符串前缀猜测。
- [普通 Lua 调用栈整改](2026-09-12-recursive-call-remediation.md)：四条 Lua 调用路径迭代化，返回 continuation 跨暂停保留，活动帧预算防止暂停绕过上限。
- [GC 回调与栈窗口整改](2026-09-12-gc-semantics-remediation.md)：自动 finalizer 状态、物理初始化窗口、native spare 槽、setmetatable 别名、宿主 helper 临时区及 set_top。
- M0 对既有 101 个 non-official case 固定执行合同，只容许 10 个已观测的具体
  stdout 差异签名；随后 metamethod-arith 修复并整条移除，剩余 9 项。新增失败、异常退出、换用例、变更通道或禁用比较均失败。
  修好的 debt 必须从许可中移除。该清单冻结历史证据，不表示所有差异已获批准。
- env loader fixture 补齐自己的 tostring 依赖，所有断言在四个引擎中实际执行。
  quality 在 nextest 之外显式执行 doctests。

## 已取得的验证证据

普通调用整改后的最终源码已执行完整 M0/M1；下表单列早先及本轮的定向 Miri 快照。
测试清单共有 1,009 项普通测试，全部执行、无 ignored，另有 10 项 doctest。
Debug/Release 都执行全部普通 targets；doctests 在独立步骤运行，不重复算作每种构建的执行数。

| 检查 | 最近结果 | 本地证据 |
| --- | --- | --- |
| 最终完整 M1 / quality | `foundationPassed=true`、0 hard failure，117,162 ms；fmt/Clippy/Debug/Release/doctests/rustdoc/audit 全部通过 | `target/recursive-call-evidence/m1-verified.json` |
| 最终完整 M0 | `passed=true`、`fullGatePassed=true`、0 hard failure，123,673 ms | `target/recursive-call-evidence/m0-verified.json` |
| 最终 workspace 测试清单 | 普通 tests 1,009（无 ignored）；doctests 10，完整 quality 独立执行通过 | `target/recursive-call-evidence/test-list.log`、`m1-verified.log` |
| 迭代帧 / GC / coroutine / iterator 返回 | 本轮默认 Miri 1/1，227.84 秒 | `target/recursive-call-evidence/miri-iterative-frames.log` |
| 普通调用进程边界 | Debug/Release 的 error_provenance 各 4/4，多栈 target 各 2/2（driver 内 1/2/8 MiB 均通过） | `target/recursive-call-evidence/test-results.json` |
| Runtime public access / 源码执行 / 结果消费 | 默认 Miri 4/4 | `target/audit-2026-09-12/runtime-access-miri-3.log` |
| Managed write / unwind / fixed child / 交错 GC 阶段 | 默认 Miri 5/5 | `target/audit-2026-09-12/gc-write-miri-final.log` |
| 非 typed IO userdata 拒绝 | 默认 Miri 1/1 | `target/audit-2026-09-12/typed-io-miri.log` |
| 完整 stdlib 初始化、native call、pcall、coroutine、gsub、sort、GC、结果消费和 close | 默认 Miri 1/1，147.92 秒；无整数转指针警告 | `target/audit-2026-09-12/stdlib-boundary-miri-4.log` |
| GC 墓碑不保活/不解引用 | 默认 Miri 通过 | Table 定向测试 |
| 表删除/同键和换键 100k churn | Debug/Release/固定 C++/官方一致 | `target/audit-fixes-2026-09-12/table-release-probes.json` |
| 486 次浮点算术、比较与零键矩阵 | Debug/Release/双 oracle 一致 | `tests/characterization/floating-point-matrix.lua` |
| rawset/os.time/next 进程边界 | Debug/Release 各 3/3 | `crates/lua_app/tests/stdlib_error_boundary.rs` |
| string 进程边界 | Debug/Release 各 3/3；既有集成 19/19 | `crates/lua_app/tests/string_error_boundary.rs` |
| IO Runtime close | Debug/Release 原 4 项通过，未改数量断言 | `crates/lua_stdlib/tests/stdlib_integration_tests.rs` |
| 最终 non-official / policy | 92 执行，83 match / 9 debt；无异常退出/超时/runner error；21 场景策略自检通过 | `target/recursive-call-evidence/m0-verified/non-official.json` |
| GC 状态/弱表/根窗口 + 最终 helper/栈 API | 默认 Miri 5/5（711.68 秒）及 3/3（122.32 秒），重叠 1 项，合计 7 个独立用例 | `target/audit-fixes-2026-09-12/gc-semantics-miri-fixed.log`、`gc-semantics-host-miri-final.log` |
| REPL completion | 原 Debug/Release 各 8/8；最终完整 M0 再次通过 | `target/error-provenance-evidence/repl-*-explicit/report.json` |
| RustSec 依赖审计 | workspace 20 / runner 34 packages：0 已知漏洞，无 warning；数据库更新 2026-09-09 | `target/audit-2026-09-12/remediation-audit.json`、`remediation-runner-audit.json` |
| 官方/性能工具 | Python 7 项测试通过；必选 case 不能通过改为 helper/excluded 跳过 | `tools/tests/` |
| Runtime 性能与托管内存 | 四类负载 × 100 批 × 2 轮，共 800 批全部完成，checksum 一致；源码冻结后独立构建，门禁结束后采样 | `target/audit-fixes-2026-09-12/runtime-baseline-final.json` |
| 静态 heap / mutation / root inventory | 60 paths / 8 families / 24 kinds 通过 | `target/audit-2026-09-12/remediation-*-contract.json`、`remediation-root-inventory.json` |

旧 unsafe 探针和前几轮 Miri 失败日志保留为修复前证据，不作为通过证明。
默认 Miri 通过也不等价于整个运行时不存在未定义行为。

## 新发现与下一步

官方 suite 之前主要有库存和编译证据，不能解释成所有文件执行通过。独立 source
lane 会记录每个文件的 run/helper/excluded 分类、参考实现是否自己失败、具体返回
值/错误/超时与到达终点的证据；原 all.lua dump/undump 路径仍有 M3 前置依赖。

初步执行发现的连续 tmpfile 假 yield、closure/events/files 根扫描拒绝和 weak kv 临时对象保留已修复，未降低任何根扫描安全检查。官方 source 完整快照为 Rust 11/19 完成、5 个失败、3 个超时；C++ 19/19，官方 Windows 17/19。具体文件与 snapshot 差别见[官方执行报告](official-source-lane.md)。本轮脚本 binary 为 core 修复后、后续 REPL completion 修复前；没有把后来 binary 冒充为该轮实测对象。

既有非 GC 计数差异需要区分具体原因：数字显示有两 oracle 的 14 位有效数字依据；
C++ 的 pcall 会补错误位置、resume 通道却不补；C++ print 对部分 userdata 输出
`unknown` 并绕过 tostring。不能用全局文本归一化消掉这些差异，也不能未经决策把
它们写成已批准 deviation。原始观察见 `target/m2-readonly-20260912/assessment.md`。

README/plan/backlog 的“当前状态”已统一到带源码版本、命令和产物的摘要。
门禁结束后，已采集长寿命表、闭包/协程、分配密集脚本的执行耗时、显式完整 GC
耗时和托管内存基线。自动 GC 暂停上界、内存配额和可恢复 OOM 仍未得到证明。

## 性能基线与解释

完整口径、原始样本和重现命令见 [Runtime 性能基线](runtime-performance-baseline.md)。
Release 使用每批 256 次操作、128 项存活窗口、5 批预热、100 批测量、2 次独立进程；
四类负载合计 800 批通过。本机采样在完整 M0/M1 和其他已知构建/测试进程结束后开始。

| 负载 | 两轮显式完整 GC p99 / ms | GC 后托管存活字节范围 | 生命周期托管峰值 / bytes |
| --- | ---: | ---: | ---: |
| 同键删除重插 | 0.0757 / 0.0659 | 101828 | 101964 |
| 固定存活窗口持续换键 | 0.0497 / 0.0618 | 99528–104848 | 130656 |
| 闭包/协程 | 0.4983 / 0.5872 | 582738 | 1174946 |
| 分配压力 | 0.4895 / 0.6837 | 264202–264458 | 532168 |

持续换键在本次负载中保持有界容量，没有再现旧实现随累计插入次数增长的趋势；
这不是对任意工作负载无泄漏的证明。第二轮分配压力保留执行 p99 **8.8689 ms**、
最大 **12.7557 ms** 的尾部样本，应先定位原因，再决定调度优化；不能仅凭耗时
将尖峰归因于 GC。

这里的 GC 是批次之间主动调用的完整回收，内存是 allocator 的托管记账，
**不是自动 GC pause 或进程 RSS**。生命周期 peak 包含初始化、编译和预热。
8 次 Runtime close 均返回 Ok，但本版 JSON 没有展开关闭报告，不能据它声称关闭后零字节。

冻结源码清单 SHA-256 为
`2e9098fe0bc6050867380b425bd90905f0fe39160186fae715145ffbe7096173`；
实际执行二进制 SHA-256 为
`c3e5798f080a348346bff5b3fb9b6febb1d29f02c7309b4dfbcf6585c6babd12`。
该快照已包含最终 REPL 补全与 GC 测试夹具修正，和官方 19 项执行快照分开记录。


## 新开放风险与交付顺序

先前普通非尾递归在 Windows Debug 中触发 `0xC00000FD`。本轮已用迭代调度移除
普通 CALL、`__call`、Lua iterator 与 root Lua tailcall 的 Rust 递归执行链；相应回归
已启用，Debug/Release 和 1/2/8 MiB 宿主线程栈均受控返回且继续执行。详见
[普通调用整改](2026-09-12-recursive-call-remediation.md)。512 活动 Lua 帧是目前保留
的 Rust 限制，完整的跨引擎资源预算与 native 重入合同仍待核实；不外推任意宿主栈安全。

官方 db 的 hook 返回顺序、后续 traceback 和 errors 原始诊断字节已修复，两个
原版文件在 Debug/Release 完成，嵌套 codegen 错误传播 P1 也已单独验收。
main 的 binary chunk 依赖、Windows files 行为以及
GC/sort/verybig 负载继续单列。M2 的 9 个历史
non-official debt 是 2 个 userdata print 和 7 个内存
计量差异；普通 arithmetic debt 已关闭。新的 callback 错误定位、xpcall/tostring 的
双 oracle 冲突不通过扩大文本 normalization 处理。

下一批：本地时间/UTC 转换 → 其余 native diagnostic 逐入口迁移 → 在基准上
评估自动 GC 调度 → [M3 格式/ABI 合同](../rust_migration/m3_contract_design.md)。
两个 oracle 的 dump 使用不同 payload；首批应先实现受限 reader/verifier 和原子发布，
再接 serializer/Lua 入口，最后公开 C ABI 与动态库生命周期。

| 优先级 | 下一批交付 | 完成标准 |
| --- | --- | --- |
| 1 | 普通 Lua 调用的宿主栈边界 | 已验证：递归回归启用；Debug/Release、1/2/8 MiB 宿主线程栈通过，超限可捕获并继续；tail call、hook、coroutine、iterator 和 GC 定向及完整门通过 |
| 2 | 官方 suite 的逐项兼容收口 | db/errors 原版与嵌套 codegen 错误传播 P1 已验收，main 已越过交互段；Windows files/popen 已定向验收，下一步处理本地时间/UTC 转换和其余平台行为；calls/main 的 dump 依赖单列到 M3，不以放宽断言或延长超时冒充修复 |
| 3 | 资源与大负载定位 | 为自动 GC 安全点补充实际工作量与耗时观测；定位 gc/sort/verybig 超时及分配压力尾部，使用同一源码冻结流程比较；为配额和 OOM 明确可恢复边界 |
| 4 | M3 首批实现 | 先完成两种 chunk 格式的显式识别、受限解码、字节码 verifier 和失败时原子回滚；在调用/异常/生命周期合同通过后再暴露 C ABI |

两个 oracle 的 print、callback/xpcall/tostring 冲突需要项目维护者选定兼容目标，
最小复现和决定进入 deviation log 后才能关闭；它们不阻止其余已定位失败的修复。
本轮 `RuntimePartsMut`、`GcWrite` 和 typed userdata 收紧了公开 Rust API，外部嵌入方
需要迁移调用；仓内编译和测试通过不等价于未知外部使用方已完成迁移。


完整门首次复验还发现两个 GC unit 的夹具把 `set_top(0)` 当成物理栈/帧退役。
它们现显式清槽和 reset 已退役的帧，保留原回收/标记数量断言；新增收缩逻辑 top 后
高位 caller 根仍存活的中间收集，随后显式退役才回收。VM lib Debug/Release 89/89。

上一轮 `target/audit-2026-09-12/` 中的 `remediation-m1-final` / `remediation-m0-final`
是失败记录，`remediation-*-verified` 是在 REPL 修复期间停止的运行；上一轮最终通过
产物为该目录的 `remediation-*-complete.json`。这些历史文件不被覆盖，也不与本轮
`target/recursive-call-evidence/` 的同类文件混用。基础门通过并未关闭公开 allocator、
自动 GC 暂停上界和全部资源预算合同。

本轮新增的 `target/recursive-call-evidence/m1-complete.json` 为失败历史：源码静态
检查把普通 `Result::as_ref()` 也列入禁用文本。已改为等价模式匹配，未放宽 guard；
最终验收使用该目录的 `m0-verified.json` 与 `m1-verified.json`。测试和完整门通过仍
不将 `m1Complete=false` 改成完成。源码与 fixtures/tools/CI 共 360 文件归档在
`target/recursive-call-evidence/source-final.zip`，清单 SHA-256 为
`dc58a49beff50b57a21c953645596ff8a232b29d4ba7043b38115cfd8a333edf`。
性能表保留普通调用整改前的独立基线，未用本轮代码冒充重测结果。
