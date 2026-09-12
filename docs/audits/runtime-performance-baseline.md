# Runtime 延迟与托管内存基线

本入口用于审计步骤 4 的可重复工作负载观察。它使用宿主 `std::time::Instant` 测量同一 Runtime 的多批执行及显式完整 GC，记录 GC 前后对象数、记账字节和 allocator 的托管峰值。**这些数据不是自动 GC pause 分布，也不是进程 RSS。** 当前没有性能倍率、跨引擎结论或自动通过的性能阈值。

工具为 [run_runtime_baseline.py](../../tools/run_runtime_baseline.py)，执行器为 [runtime_baseline.rs](../../crates/lua_app/examples/runtime_baseline.rs)。仅新增 example、Lua 工作负载及工具，不改变生产 Runtime 的执行器、GC 调度或记账。

## 可重复入口

在项目根目录执行。Windows 使用 `py -3`；其他平台使用 Python 3，并选择本机 Rust target。

```powershell
# 小规模验证：冻结当前源码，构建 Debug，再运行两个独立进程。
py -3 tools/run_runtime_baseline.py --profile dev --iterations 8 --width 4 --batches 3 --warmup 1 --repeats 2 --artifact target/performance/runtime-smoke.json

# 正式候选：先确认单批预算，再采集每进程 100 个测量批次。
py -3 tools/run_runtime_baseline.py --profile release --iterations 256 --width 128 --batches 100 --warmup 5 --repeats 3 --artifact target/performance/runtime-release.json

# 可单独放大长寿命表观察，所有操作仍在同一个活表上进行。
py -3 tools/run_runtime_baseline.py --profile release --workload table-rotate --iterations 1000 --width 128 --batches 100 --warmup 5 --repeats 3 --artifact target/performance/table-rotate.json
```

`iterations` 是每批主要循环次数，`width` 是持续存活窗口，`batches` 不含 warmup。每个进程按固定顺序执行所选负载，各负载创建一个新的 Runtime；该 Runtime、编译后显式 root 的 Proto 和 Lua 存活对象跨所有 warmup/测量批次复用。`repeats` 创建独立宿主进程，便于观察运行间差异。编译和 setup 不计入批次执行耗时。

默认每进程超时 300 秒、构建超时 900 秒，可用 `--timeout-seconds`、`--build-timeout-seconds` 调整。超时及非正常退出作为可见失败写入 JSON，终止进程树。不要为了缩短时间删除慢样本；需要缩小时另建一份带新参数的结果。采样期间应减少并发编译、测试和其他机器负载，记录任何无法避免的竞争。

## 工作负载与存活合同

| 名称 | 持续状态与每批操作 | 检查 |
|---|---|---|
| `table-reinsert` | 同一个活表与固定的负整数 hash key 集合；逐项删除后重插 | 活跃键数始终等于 width，所有登记键仍存在，返回值累加 checksum |
| `table-rotate` | 同一个活表，活跃键数固定；每次删除旧负整数键并插入一个从未使用的新键 | 跨批次保留表；检查数量、映射及 checksum，观察墓碑压缩后的容量周期是否有界 |
| `closure-coroutine` | width 个闭包和挂起协程跨批次/GC 存活；每次恢复并更新；同时产生短命协程及捕获局部表的闭包 | 持续闭包与协程计数一致；短命闭包在 yield 与协程返回后均可读取正确 upvalue |
| `allocation-pressure` | 循环产生嵌套表、数组及包含唯一编号的字符串，只保留最后 width 项 | 检查存活窗口大小、内容与 checksum；释放旧对象形成分配压力 |

Lua 源码在 [workloads](../../crates/lua_app/examples/workloads) 中，经 `include_str!` 编入 example。参数与 checksum 的整数运算范围受到检查，避免超出 binary64 精确整数范围。工作负载不使用 `os.clock`、随机数或外部 IO。

## 计量口径

每批依次采集 `before_execution`，用 Instant 包围 `execute_proto_with_args`，采集 `before_explicit_gc`，用 Instant 包围 `Runtime::collect_garbage_full`，最后采集 `after_explicit_gc`。显式 GC 在释放 RuntimePartsMut 借用后的空闲宿主边界执行，保留 Runtime 的规范 root 扫描与状态退休规则。

- `execution_wall_ns` 包含 VM 调用、参数/返回值处理、负载断言，以及生产默认自动 GC 在执行中发生的工作；不包含源码编译、统计快照和批次末尾的显式 GC。
- `explicit_full_gc_wall_ns` 包含完整宿主回收方法的调用开销及实际回收工作；它没有为自动 GC 安全点插桩。显式调用每批一次，会改变自然分配周期，不能据此推算真实业务的自动 GC pause。
- `gc_objects` 是 collector 当前管理的对象数；`gc_accounted_bytes` 是 collector 的估算对象记账。
- `managed_live_bytes`、`managed_lifetime_peak_bytes`、`managed_lifetime_allocated_bytes` 来自 allocator ledger：对象 layout、托管容器 capacity、StringPool key 缓冲及 Runtime LuaState 等。范围不含宿主 allocator metadata、其他 Rust 服务开销或 OS RSS。
- peak 与 allocated 均为该 Runtime 生命周期累计值，包含 stdlib 初始化、编译、setup 和 warmup；不是可重置的单批峰值。warmup 及 setup 的 GC 耗时不进入测量分布。
- `after_release_and_two_full_collections` 是 Lua 负载移除自身全局引用后两次完整回收的观察，Proto 显式 root 此时仍保留，宿主容器可能保留容量。不能要求它与 setup 前逐字节完全相同。

分布使用最近秩：将 N 个观测排序，取 `ceil(p*N/100)-1` 的样本。JSON 保留所有原始批次、min/max/mean 与 p50/p95/p99。N=3 时 p95/p99 都是最大值，只能验证流程；N=100 也只是有限样本的尾部观察，不能宣称严格暂停上界。持续编号的位数增长、表容量阶跃等可能形成有界变动，单次高低点不能独自证明泄漏。

## 源码与二进制身份

工具先通过 Git inventory 收集当前 Cargo workspace、`.cargo`、toolchain、全部 crate 文件及两个工具文件；包括尚未提交但未被忽略的源码。它复制原始字节，逐文件 SHA-256，前后复核以拒绝复制期间的并发编辑，再保留 ZIP 与文件清单的独立 hash。

Cargo 使用 `--locked`、明确 target/profile 和独立 target 目录，**直接从复制的源码构建**。构建后再次核查源码 hash，根据 Cargo artifact 信息取得 example，复制可执行文件及相邻 DLL 到独立目录，记录实际复制字节的 SHA。所有重复调用同一份二进制快照，并在每次运行前核查 SHA。工作树后续重编译不会替换正在测量的程序。

JSON 同时保留原 Git HEAD、脏工作树状态、实际 rustc/cargo 版本、构建命令、相关环境覆盖、宿主 OS/CPU 信息、源文件/ZIP/binary/DLL hash 及 stdout/stderr 路径。Git HEAD 是上下文标记；**不能用它替代脏工作树 source manifest**。该流程没有声称对系统链接器、Rust 安装或宿主环境做了完整 hermetic 封装。

工具退出码：0 表示全部所选负载完成检查且报告完整；1 表示运行失败、超时或结果合同不完整；2 表示配置、源码固定、构建或工具错误。启动时先写 `complete:false, passed:false`，防止中断报告被当作成功结果。

## 当前验证与集成

2026-09-12 已完成原生 Debug 小探针：四个负载，8 次操作、width=4、1 次 warmup、3 个测量批次，连续两进程通过。此数据只验证工具链，来源为当时的中间工作树，不作为 Release 基线。

同日 M0/M1 门禁结束、其他已知构建/测试进程退出后，完成正式 Release 采集：`iterations=256, width=128, batches=100, warmup=5, repeats=2`。两个进程均 exit 0，四负载共 800 个测量批次全部完成；两进程对应负载的 checksum 序列一致。使用 Windows 10 build 19045、Intel64 Family 6 Model 183 Stepping 1（20 logical CPUs）、rustc/cargo 1.96.1、`x86_64-pc-windows-msvc`、release/LTO。两进程总耗时分别约 1.231、1.174 秒，不含构建。

- 完整结果：`target/audit-fixes-2026-09-12/runtime-baseline-final.json`；便于阅读的逐轮摘要：`runtime-baseline-final-summary.json`。
- Source manifest SHA-256：`2e9098fe0bc6050867380b425bd90905f0fe39160186fae715145ffbe7096173`。
- 实际执行 binary SHA-256：`c3e5798f080a348346bff5b3fb9b6febb1d29f02c7309b4dfbcf6585c6babd12`。
- 该源码包含随后 REPL 补全访问迁移和 VM 测试 fixture 修正；与官方 19 项的 `fc06…` 源码脚本测试快照须分开标记。

以下耗时单位均为 **ms**，每行基于该进程的 100 个样本，保留独立重复而不合并掩盖波动。

| 负载 / 轮次 | 执行 p50 | 执行 p95 | 执行 p99 | 显式完整 GC p50 | GC p95 | GC p99 |
|---|---:|---:|---:|---:|---:|---:|
| table-reinsert / 1 | 0.3975 | 0.4125 | 0.4849 | 0.0439 | 0.0536 | 0.0757 |
| table-reinsert / 2 | 0.3961 | 0.4413 | 0.6186 | 0.0409 | 0.0547 | 0.0659 |
| table-rotate / 1 | 0.4589 | 0.4806 | 0.4938 | 0.0397 | 0.0449 | 0.0497 |
| table-rotate / 2 | 0.4672 | 0.5005 | 0.5732 | 0.0408 | 0.0548 | 0.0618 |
| closure-coroutine / 1 | 6.2130 | 6.7009 | 6.8523 | 0.3259 | 0.3931 | 0.4983 |
| closure-coroutine / 2 | 6.2688 | 6.8808 | 7.1643 | 0.3308 | 0.4301 | 0.5872 |
| allocation-pressure / 1 | 2.5062 | 2.7074 | 3.2163 | 0.3980 | 0.4373 | 0.4895 |
| allocation-pressure / 2 | 2.4647 | 3.1814 | 8.8689 | 0.3916 | 0.4892 | 0.6837 |

| 负载 | 两轮 GC 后 managed live 范围 / bytes | GC 后对象数 | 测量结束时累计 managed peak / bytes | 移除负载引用并两次 GC 后 managed live / bytes |
|---|---:|---:|---:|---:|
| table-reinsert | 101828–101828 | 364 | 101964 | 75532–75588 |
| table-rotate | 99528–104848 | 364 | 130656 | 75528–75584 |
| closure-coroutine | 582738–582738 | 1009 | 1174946 | 79570 |
| allocation-pressure | 264202–264458 | 1007 | 532168 | 76362 |

原键重插的存活记账固定；持续换键在本次 100 批中有界波动，首末分别为 103000→102216、104680→104288 bytes，没有旧实现随累计插入次数增长的趋势。分配负载的编号位数增加伴随少量字符串记账增长，对象数保持不变。该观察不代替更大 width、更长持续时间和真实嵌入应用的验证。

第二轮分配压力存在真实尾部样本：执行最大 12.7557 ms、显式 GC 最大 1.6227 ms，均未删除或重跑覆盖。p99 与 max 不相同，100 个样本也不能识别所有罕见停顿。下一轮应在固定机器上增加持续时间和规模，并针对自动 GC 安全点另行计时；不要据这些显式调用数据承诺自动暂停上界。

8 个 Runtime 均执行了显式 `close()` 并返回 `Ok`（失败会使该负载和进程失败）。本版 JSON 没有展开 `RuntimeCloseReport` 的各剩余计数字段，因此不能据此宣称 close 后为零字节；表中最后一列是 **close 前** 的释放/回收快照，close 耗时也不进入测量分布。

```powershell
cargo test -p lua_app --example runtime_baseline
cargo clippy -p lua_app --example runtime_baseline -- -D warnings
py -3 -m unittest discover -s tools/tests -p test_runtime_baseline.py -v
```

Rust 单测检查最近秩尾部取样；Python 测试检查源码字节冻结、并发编辑拒绝及完整结果合同。端到端小探针覆盖 Lua 断言和 Runtime 生命周期。CI 可保留小规模冒烟与 JSON artifact；具有统计意义的性能变化应在固定机器/配置、较大样本和多次重复上评审，暂不把任意耗时波动变成硬阈值。下一步若需要自动 GC 的暂停分布，应另行设计安全点计时及工作量指标，先与本基线分清口径再讨论调度优化。
