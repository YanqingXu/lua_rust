# v0.17 · 宿主标准库与 Lua 模块

本快照把标准库边界扩展到宿主资源：文件 userdata 有独立 metatable，并在 close、GC 或 VM shutdown 时释放；os API 统一返回平台错误。VM 直接复用编译器实现 `loadstring/loadfile/dofile` 与 Lua 文件模块。

```powershell
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml -- `
  book/code/v17-host-stdlib-modules/examples/demo.lua
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --workspace
```

`require` 支持 `package.path`、`package.preload`、`package.loaded`、缓存、循环 sentinel 与失败回滚；debug 库可查看 metatable、registry、Proto 信息和 upvalue。动态 native module 被明确排除。

## require 的三条问题链

- [缓存](exercises/cache/README.md)：两个 TODO；真值命中、只取 loader 首结果、保留自发布值；真实 VM 核对值、次数、身份和跨 GC 字段访问。
- [循环](exercises/cycles/README.md)：一个 TODO；在 loader 前发布 true，固定有限预算重放晚发布反例；真实 preload/文件分别对照。
- [重试](exercises/retry/README.md)：一个 TODO；保存 Result、错误只清当前 key；检查依赖/prefix 保留、同 VM 修文件重试与禁止 yield 后恢复。

从仓库根目录执行：

```powershell
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml -- book/code/v17-host-stdlib-modules/examples/require_contracts.lua
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_cache_problem_chain
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_cycle_problem_chain
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example require_retry_problem_chain
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cache_reference
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_reference
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_reference
```

Lua 探针应返回 true：正常模块执行一次、A/B 内层读到临时 true、失败后实际重试两次。
三个 CLI 固定调用 faults/solution，不读取 starter；默认参考通过不能代替 learner：

```powershell
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cache_exercises -- --ignored
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test cycles_exercises -- --ignored
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test retry_exercises -- --ignored
cargo fmt --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --all -- --check
cargo clippy --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --workspace --release
```

初始 learner 分别 2/1/1 项失败，只修改各包 starter 的 hook 后通过。维护者还要完成
失败→临时参考通过→原字节恢复后失败，并刷新 mtime 避免构建缓存掩盖结果。

require 参考数量分别为 cache 9、cycles 6、retry 9，与原快照 119 项合计 143 通过、
4 ignored。文件包再增加 18 项参考与 3 个 learner：全快照 **161 通过、7 ignored**。
计数仅说明默认参考和既有测试通过；七个 TODO 要用显式 learner 命令验收。

这些是独立局部错误设计，不是历史中间 VM，也不拼成完整 interpreter。对照保留当前
快照的细节：false 会再次加载；loader 首结果 nil 时重读当前 slot，若 loader 已清 slot
则本次只返回 true 不写回；循环内层命中 true 而不是完整模块对象或官方循环错误。
失败恢复只删除当前 key，不回滚成功依赖、全局副作用或 OS 资源。定位/编译错误在发布
标记之前，运行错误在发布之后。完整 Lua 兼容、panic/abort 和任意 GC 窗口另有边界，
本轮不会用 module 测试覆盖 v0.15/v0.16 已记录的 GC 功能缺口。

## 文件 owner 的三条问题链

- [close / tombstone](exercises/file_close/README.md)：一个 TODO；只改 flag 仍持有 File，直接 remove 又丢失 closed 身份；用 take 留下 None，检查别名、错误分类和跨 GC metatable。
- [GC prune](exercises/file_prune/README.md)：一个 TODO；真实 Heap 完成周期后按 is_live 清理 map，验 live/stale × Some/None 四格；key 不作 roots，Some 也不是可达性判据。
- [Drop drain](exercises/file_drain/README.md)：一个 TODO；先清文件再 shutdown Heap；固定故障只证明 shutdown 时仍持有 File，Rust 字段最后仍会自动析构。

```powershell
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml -- book/code/v17-host-stdlib-modules/examples/file_lifecycle_contracts.lua
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_close_problem_chain
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_prune_problem_chain
cargo run --quiet --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --example file_drain_problem_chain
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_close_reference
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_prune_reference
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_drain_reference
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_close_exercises -- --ignored
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_prune_exercises -- --ignored
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --test file_drain_exercises -- --ignored
```

三个新 learner 各初始失败一项，starter 独立完成。默认参考分别 close 8、prune 6、drain 4。
模型用实际 File 和 Drop 记录观察 owner 释放；真实 VM 另验对应 Lua 身份、GC 或退出行为。
Windows 的 `share_mode(0)` 探针先验证 sharing violation，再在释放后独占重开；非 Windows
明确没有同等 OS 证据。删除成功仅作清理，不能单独证明关闭。Drop 也不承诺 fsync 或
process exit/abort 清理；Heap shutdown 不执行 Lua finalizer。练习是局部模型，不是历史
中间版本；永久复现入口在这些 README 和 CLI，本机 `target/` 验证记录不随 clone 交付。

io 默认流切换/lines、完整 os locale/date、package.loadlib、debug locals/hooks/function environment 仍留待兼容收口。
