# 08 · 用测试守住兼容性

<div class="lesson-meta"><span>预计 20 分钟</span><span>Rust：单元 / 集成 / 差分测试</span><span>工程实践</span></div>

## “能运行”不是“与 Lua 5.1 一致”

自己实现的解释器打印出 `42`，只能证明一条路径可用。兼容性还包括错误文本、退出码、多返回值、NUL 字节、元方法顺序、协程状态以及大量边界行为。

项目把证据分成几层：

| 层级 | 回答的问题 | 典型位置 |
|---|---|---|
| 单元测试 | 一个数据结构或纯函数是否满足局部约束？ | crate 内 `#[cfg(test)]` |
| crate 集成测试 | 多个模块组合后是否工作？ | `crates/*/tests/` |
| Lua fixture | 从 CLI 运行脚本的可观察结果是否正确？ | `tests/lua/` |
| 差分测试 | 与官方 Lua 5.1 和固定 C++ oracle 是否一致？ | `tools/run_lua51_differential.ps1` |
| 质量门 | 格式、lint、debug/release、文档与兼容测试是否一起通过？ | `tools/*_gate.ps1`、CI |

## 为什么需要 oracle

Lua 语义文档不可能覆盖所有实现细节。差分测试把同一输入交给多个解释器，再比较：

```text
stdout · stderr · exit code · timeout · bytecode/trace（可用时）
```

官方 Lua 5.1.5 是语言行为基准，固定版本的 C++ 实现提供迁移对照。固定 commit 很关键：如果 oracle 自己每天变化，差分结果就无法复现。

## 原始差异与规范化差异

Windows 与 Unix 的行尾不同，临时路径也可能不同。测试工具可以额外提供规范化视图帮助聚类，但不能用规范化结果掩盖原始输出差异。

一个成熟报告应同时告诉你：

- 原始字节是否完全相同；
- 哪些差异只来自 EOL 或路径；
- 哪些差异是真正的语言语义；
- 超时和 runner 自身错误是否为零。

这体现了一条通用工程原则：**分析层可以降噪，验收层不能偷偷改口径。**

## 为一个修复选择最小证据

假设你修复 `Table::set` 的 NaN 键行为，合理顺序是：

1. 增加局部单元测试，锁定数据结构规则；
2. 增加 Lua fixture，锁定用户可见错误；
3. 对同一 fixture 做 oracle 差分；
4. 最后运行完整质量门，检查无关回归。

不要每改一行就只跑全部测试，也不要永远只跑一个快速单测。验证范围应随着信心扩大。

## 本地常用入口

```powershell
cargo test --workspace
cargo test --workspace --release
powershell -File tools/rust_quality_gate.ps1 -SkipAudit
```

完整兼容门还依赖固定 oracle 环境，请根据仓库 README 与工具脚本准备路径。`-SkipAudit` 只跳过依赖漏洞审计，不应被描述成完整 CI 证据。

<div class="exercise">
  <strong>动手练习</strong>
  <ol>
    <li>选择前面一章的练习，为它设计“单元测试 + Lua fixture”两层证据。</li>
    <li>阅读 <code>tests/compatibility/lua_fixtures.json</code>，找出 fixture 如何声明预期与分类。</li>
    <li>解释为什么 debug 与 release 都值得测试：整数溢出、优化和条件编译可能让行为不同。</li>
  </ol>
</div>

<div class="checkpoint">
  <strong>本章检查点</strong>
  <p>兼容性是一组可复现的外部观察，不是“测试很多”的感觉。局部测试定位规则，fixture 覆盖流程，固定 oracle 判断语言行为。</p>
</div>

继续探索：[`M0 收口报告`](https://github.com/YanqingXu/lua_rust/blob/main/docs/rust_migration/m0_report.md) · [`兼容偏差日志`](https://github.com/YanqingXu/lua_rust/blob/main/docs/rust_migration/deviation_log.md)
