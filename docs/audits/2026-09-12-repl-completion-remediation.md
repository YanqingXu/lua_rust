# REPL 点路径补全上下文遗漏修复

最终 M0 的原有 REPL 合同最初为 7/8：只有 `completion` 失败，其 stdout、stderr、history、exit 四项同时失败是一个提前退出的连带结果。原始失败保留在 `target/audit-2026-09-12/remediation-m0/parity-runner-self-test/repl-contract/report.json`。

最小输入为 `=string.su<Tab>('abcd',2)` 后接 `quit`。预期补全为 `=string.sub('abcd',2)`，打印 `bcd`，保存完成后的输入历史并退出 0；原 Rust 在首个提示符后退出 1，stderr 为 `canonical Lua string publication requires an active StringPool`，未保存历史。相邻的无 Tab 输入 `=string.sub('abcd',2)` 和全局名补全 `pri<Tab>('ok')` 均正常，固定 C++ 三种输入均正常。独立输入与原始输出见 `target/error-provenance-evidence/repl-probe/observations.json`。

根因属于 A01 动态服务边界迁移遗漏，与错误来源、`set_top` 或 GC helper 行为无关。`repl_completion::complete` 从 `RuntimePartsMut::split_mut` 获得显式 state/GC，却让点路径查找调用 `state.find_interned_string`；后者需要同时恢复动态 collector 与 StringPool。取消 parts 自动动态 scope 后，该调用正确地 fail closed。单纯换成 `with_context_gc` 也不充分，因为它有意禁止恢复动态 collector；该中间尝试的 7/8 结果保留在 `target/error-provenance-evidence/repl-debug/report.json`，没有作为通过证据。

最终修复仅修改 `crates/lua_app/src/repl_completion.rs`：从 `split_mut` 取得 StringPool，并以共享引用逐层传入补全 helper；查找使用 `StringPool::find_bytes`，随后通过显式 GC 校验字符串身份，再读取表。没有修改 Runtime 动态服务安全合同、GC 边界、REPL baseline 或预期结果，也没有新增镜像实现的测试。

使用最终编译产物的独立 snapshot 执行现有 `tools/test_repl_contract.ps1`，两构建均为 **8/8**，全部 stdout/stderr/history/exit 对齐固定 C++ 与原有期望：

- Debug：`target/error-provenance-evidence/repl-debug-explicit/report.json`。
- Release：`target/error-provenance-evidence/repl-release-explicit/report.json`。

验证子进程全部退出；没有占用供最终 gate 重建的原始 exe。最终源码文件 Git blob 为 `e87cdef5f1f11b8735a941ad381a3b80ef252371`。旧实现启动的中间 gate 结果仅保留为历史失败证据；最终统一 M0/M1 由 root 从该冻结源码重跑。
