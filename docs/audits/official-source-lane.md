# 官方 Lua 5.1 源码执行 lane

本 lane 把仓库的 24 个官方 Lua 文件逐个分类，以独立进程实际执行其中 19 个入口。它不等同于完整 `all.lua`：总入口的 dump/undump 包装、共享全局状态、最终清理、TestC 私有接口及 `big.lua` 的 >4 GiB 溢出压力尚未覆盖。

分类来源为 [official-source-lane.json](../../tests/compatibility/official-source-lane.json)。本工具不修改上游 Lua 源码或固定 C++ oracle pin。CI 已另行接入工具测试及显式选出的严格通过子集，完整 19 项的实际结果仍独立保存。

## 运行

从项目根目录运行，Windows 使用 Python launcher，Linux/macOS 将 `py -3` 换成 `python3` 并传入对应可执行文件：

```powershell
py -3 tools/run_official_suite.py `
  --rust target/x86_64-pc-windows-msvc/release/lua_app.exe `
  --official target/oracles/lua-5.1.5/build/Release/lua51.exe `
  --cpp target/oracles/lua_cpp/build/Release/lua_app.exe `
  --artifact target/compatibility/official-source-lane.json

py -3 -m unittest discover -s tools/tests -p test_official_suite_lane.py -v
```

`--case constructs.lua` 可重复使用，选择范围保存在报告中。显式选择不存在或已改为 helper/excluded 的文件会报配置错误，不能静默缩小通过范围。默认执行全部 19 个 `run` 文件；普通文件超时 30 秒，GC、sort、verybig 为 60 秒。`--timeout-seconds` 可显式调整预算，报告保存实际耗时和超时状态。

退出码 0 表示**所选 source profile** 的所有引擎都完成了测试；1 表示至少一个实际失败、超时、非正常退出或缺少完成标记；2 表示库存、来源或工具配置错误。当前兼容失败不会被 `allow-differences` 宽限。排除和辅助文件不算通过，报告始终标记 `complete_upstream_suite=false`。

## Harness 与证据

- 上游 README 要求 `LUA_PATH=?;./?.lua` 和可写的 `libs/P1`。目录、当前工作路径及子进程环境均按此准备；不能仅设置 `./?.lua`，因为 `main.lua` 用 `-l` 加载无扩展名临时文件。
- 每个引擎、每个文件有独立目录，`TMP`、`TEMP`、`TMPDIR` 也指向该目录。所有 stdout/stderr 原始字节留在 `stdout.bin`/`stderr.bin`；JSON 保留前 4 MiB 的十六进制、文本及截断标志。
- 与 `all.lua` 一致，设置 C locale、随机种子 0、GC step multiplier 180、pause 190；不设置 `_soft`。source profile 通过 `loadfile` 执行原始文件，不做 dump/undump。
- 运行开始时快照全部源码和可执行文件，记录源码、引擎副本、邻接 DLL 和 runner SHA-256。即使另一个终端重新构建原可执行文件，本轮仍使用同一副本。
- 来源验证使用现有 `lua51-official-sources.json`。Windows checkout 的 CRLF 仅在计算上游 LF hash 时规范化；执行的是工作树原始字节副本。
- 主脚本正常返回后才打印独有的完成标记，并检查 `calls`、`attrib`、`locals`、`events`、`verybig` 的上游返回合同。只看到 `OK` 或退出码 0 均不足以通过。
- 超时会终止子孙进程；runner 测试验证了此路径、异常退出分类、缺少完成标记、来源变动和中途重建二进制的隔离。
- 上游条件跳过信息与 `partial_exclusions` 分别记录。`api.lua` 和 `code.lua` 在无 T 时立刻返回，明确排除，不能将这种返回计为有效测试。

现有 provenance 除 24 个 Lua 文件，还列出 README、`etc/ltests.c/.h`、五个 native-library 源码/构建文件和 `libs/P1/.gitkeep`；这些九个辅助文件不在当前 tracked subset 中。报告记录缺失清单。上游 README 已从固定 C++ checkout 的相同 suite 读取核实；本 lane 没有下载、切换或重新指定 oracle。

## 24 个文件的分类与已完成的 Release 轮

2026-09-12 core 安全/语义修复后的完整轮使用 Rust 副本 SHA-256 `fc06b34e4a2ab22729dcc5d3979fa0ccd78ecfc416d30db84ade7da0c327d886`。它早于随后宿主 REPL 点路径补全的 `with_context_gc` 迁移及两个 VM 测试 fixture 修正；这些后续变化不经过本轮源码脚本执行路径，但本轮仍只作为所记录 binary SHA 的实测证据。Artifact：`target/audit-fixes-2026-09-12/official-source-final.json`。19 项全部实际运行：**Rust 11 完成、5 失败、3 超时；固定 C++ 19 完成；官方 Lua 5.1.5 在本机 Windows 上 17 完成、2 失败。** Runner 如实返回 1，`complete:true, passed:false`。

本轮与 quality/M0 门并行运行，机器存在测试/构建负载；三项 60 秒超时是该预算下未完成，不证明死循环，也不是空闲机器的性能结论。后续空闲机器的定向观察应另存结果，不能覆盖本轮原始 timeout。

历史中间轮 `official-source-verified.json` 的 Rust SHA 为 `dbc038f8077139026d7567f3b4784a24ef6d87dae7cf9def0a10162706ba7e65`，当时 8 完成、9 失败、2 超时；它早于 GC callback/物理栈窗口等修复，仅用于追踪改善，不作为最新源码状态。

| 文件 | 分类 | 最新 Rust 观察 / 排除原因 |
|---|---|---|
| all.lua | helper | 上游总编排；本 lane 展开源码入口，不冒称覆盖其 dump/undump、共享状态和最终清理。 |
| api.lua | excluded | 需要 TestC `T.testC` 等私有 API；三个 stock 引擎均不提供。 |
| attrib.lua | run | 完成并验证返回 27；动态 native-library 子块因缺少编译库被上游跳过。原 GC callback 错误已越过。 |
| big.lua | excluded | 第一段将 32 MiB 字符串连接 129 次，依赖 >4 GiB 溢出及随后协程 b/a 返回合同；需有明确内存预算的专用 lane。 |
| calls.lua | run | 到达 `string.dump`，产品明确报告 binary chunk serialization 未实现。 |
| checktable.lua | helper | 为 math/big/verybig 定义 `stat`；无 TestC 时仅安装提示用的 fallback。 |
| closure.lua | run | 完成；T yield/hook 子块另有明确排除。原 root trace unsafe gap 已越过。 |
| code.lua | excluded | 全文件依赖 `T.listcode`；现有 bytecode parity 属于其他测试，不能算此文件执行通过。 |
| constructs.lua | run | 完成。 |
| db.lua | run | `wrong trace!!`；诊断包装定位到第 17 行，期望 line 序列耗尽后仍收到 line=19。 |
| errors.lua | run | 第 196 行的 `checksyntax("\255a = 1", ..., "\255", 1)` 在 helper 第 20 行失败：Rust 诊断把原始 0xFF 写成字面 `\xff`，两个 oracle 均保留 raw byte。旧 `global 'insert'` 等错误来源段落已越过。 |
| events.lua | run | 完成并验证返回 12。原 root trace unsafe gap 已越过。 |
| files.lua | run | 第 271 行 `assert(y:close())` 失败：`io.popen("ls")` 的 Windows 命令不存在，随后 close 返回失败。官方 Windows 更早在第 59 行 `os.rename` 返回 Permission denied；固定 C++ 完成。原 root trace 失败已越过。 |
| gc.lua | run | 60 秒超时，已打印 `deep structures`，越过原 weak tables 断言；200k 深结构负载未在预算内完成。 |
| literals.lua | run | 完成；pt_BR/ptb locale 不可用时的上游条件跳过单独记录。 |
| locals.lua | run | 完成并验证返回 5；私有 querytab 子块不在 stock profile 中。 |
| main.lua | run | `-i` 多行 `a` / `= 10` 的续读合同失败，输出 6/10/nil/nil 而期望 6/10/10。官方 Windows 的临时文件名带 `.0`，require 将点转目录分隔，较早的 `-l` 临时模块失败；固定 C++ 完成。 |
| math.lua | run | 完成完整源码，未用 `_soft` 跳过 NaN、随机数及文件段落。 |
| nextvar.lua | run | 完成；私有 table sizing/int2fb 的 if-T 块不在 stock profile 中。 |
| pm.lua | run | 完成。 |
| sort.lua | run | 60 秒超时；随机排序和重排已打印完成，尚未完成 Lua 比较器的反向排序。超时不是已证明死循环。 |
| strings.lua | run | 完成。 |
| vararg.lua | run | 完成。 |
| verybig.lua | run | >64k 源码/70001 字段负载在 60 秒内未完成；未通过 `_soft` 将整个文件跳过。 |

db 的细定位来自历史独立诊断包装；其最新原始错误仍为 `wrong trace!!`。errors/files 的最新定位与短探针保存在 `official-final-diagnostics.json`，使用同一 Rust binary SHA，包装仅输出失败位置/局部变量，原源码未修改。这些诊断不是通过证据。

保留独立的 oracle 分歧：Windows `io.popen("cmd /c exit 7"):close()` 在 Rust 与固定 C++ 均返回 nil/string/number，而官方 Lua 5.1.5 返回 true；不能把完整 files.lua 的平台命令失败简化成三个引擎共同的关闭返回合同。主 JSON 保留原始字节，非法 UTF-8 在辅助可读文本中的替换字符不代表真实输出字节。

## 集成建议

1. 完整 19 项冻结轮已完成；保留 source/binary/runner hash 和原始失败 artifact。对压力 timeout，在空闲机器上另做有明确预算的定向观察。
2. CI 已接入 runner 的五个工具测试及原八个稳定通过入口。新增 attrib/closure/events 在本轮三个引擎均完成，可经复核扩入严格子集。完整 19 项保持独立结果，任何失败都返回非零，不能因“已登记官方库存”变成通过。
3. 在 Linux 增加 stock 官方基准，单独处理 Windows `main.lua`/`files.lua` 的非可移植行为。保留两 oracle 的分歧，不把 stock 自身失败归成 Rust 回归。
4. 依次处理剩余 debug trace、错误文本字节合同、交互续读、平台 IO、binary chunk 和长负载问题。只有相应原文件真正到达完成标记，才将其加入通过清单。
5. 后续分别建立有内存约束的 `big.lua` 协程/溢出 lane、TestC 内部测试 lane、以及完整 `all.lua` dump/undump/共享状态/清理 lane。更换测试能力时保持 oracle pin 与来源审批制度。
