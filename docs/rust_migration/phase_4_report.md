---
status: partial
phase: 4
phase_name: Standard Library
last_updated: 2026-07-26
rust_baseline: 62841357939b2992a8e5eaf715d603c11d2d6a2d
cpp_oracle: 87c15e69ceb94eb74e28226ccbefb7e196635711
---

# Phase 4 Report: Standard Library

## 结论

**状态：partial。** base、math、string、table、io、os、coroutine、debug 和 package
模块均有注册入口及项目内集成测试；多个模块仍有明确近似、unsupported 分支或缺失
函数，官方 Lua 5.1 strict suite 也没有通过证据。

| 分类 | 本阶段内容 |
|---|---|
| `completed` | 无。模块注册和项目内测试尚不足以满足标准库阶段 oracle 门。 |
| `partial` | base/math/string/table/io/os/coroutine/debug/package 的已注册实现。 |
| `not-started` | 真正 native module loader，以及首批缺失的 C++ 扩展/API 注册面。 |

## 证据矩阵

| 能力 | 实现证据 | 验证证据 | 判定 |
|---|---|---|---|
| Library catalog | `crates/lua_stdlib/src/catalog.rs` 注册 9 个模块/命名空间 | stdlib integration tests | partial：注册不证明每个 API 的完整行为 |
| Base/math/table | 对应 `base.rs`、`math.rs`、`table.rs` | stdlib tests 与项目 fixtures | partial：错误文本、GC、扩展函数和边界未全量对齐 |
| String/pattern | `string.rs` 含 pattern/format/dump 路径 | stdlib tests | partial：byte semantics 和 binary dump 不兼容 |
| IO | `io.rs` 含 file userdata 与 read/write/seek 等 | 项目内 IO tests；当前 M0 进程测试与 4-case 双 oracle 本地通过 | partial：修复待 CI/合入，byte/path 与完整 IO 边界未完成 |
| OS | `os.rs` 含 clock/date/time/execute/file operations | C locale/UTC 项目内子集测试 | partial：实现本身是已登记近似 |
| Coroutine/debug/package | 对应模块存在并有主要路径 | stdlib integration tests | partial：native module、debug matrix、生命周期不完整 |

## 未完成与阻塞

| 阻塞 | 现场证据 | 跟踪 |
|---|---|---|
| `_VERSION` 与 stock 不同 | 项目目标跟随 C++ 扩展值 | [NOTE-001](deviation_log.md#note-001-项目扩展-_version-值)，M0.5、M2.8 |
| GC API 为模拟/半闭环 | count/step 不是实际内存，collect 不 sweep | [NOTE-002](deviation_log.md#note-002-gc-可观察行为尚未形成真实回收闭环)，M1.9–M1.12、M2.9 |
| `string.dump` / binary chunk 未实现 | M1.6 已删除 thread-local ID registry，并改为显式 unsupported | [NOTE-003](deviation_log.md#note-003-stringdump-不是-lua-51-binary-chunk)，M2.10、M3.1–M3.3 |
| 默认 stdio 基线不可观察，当前修复待集成 | 本地 process tests 与双 oracle 4/4 已通过；完整 fixture/CI/合入仍待验收 | [NOTE-004](deviation_log.md#note-004-默认标准流从-memory-file-迁移到宿主流)，M0.5、M2.12 |
| Native module unsupported | `package.loadlib` 直接返回 unsupported | [NOTE-005](deviation_log.md#note-005-packageloadlib-明确不支持动态库)，M2.15、M3.9 |
| OS/locale/time 为近似 | wall-clock、UTC-only、C-only locale 等 | [NOTE-006](deviation_log.md#note-006-oslocale-与-time-使用平台无关近似)，M2.13 |
| String API 不能完整保留任意 bytes | 核心为 Rust `String` 并跨 char/byte shim 转换 | [NOTE-007](deviation_log.md#note-007-lua-字符串尚未采用任意字节表示)，M1.1–M1.3、M2.10 |
| 若干 C++ 项目扩展/首批 API 未注册 | `table.pack/unpack/move`、`io.popen`、`os.exit/getenv`、`debug.debug/getmetatable` | M2.8 |

## Oracle 与验收

- Stock API 语义比较官方 Lua 5.1.5；项目扩展、错误分类和平台策略比较
  `lua_cpp@87c15e6`。
- 现有 stdlib integration tests 只证明当前实现的项目内合同。它们不能批准
  NOTE-002/003/005/006/007，也不能替代 official strict/slow suite。
- 完成条件对应 M2.8–M2.15 及 M2 总门槛；目前保持 `partial`。
