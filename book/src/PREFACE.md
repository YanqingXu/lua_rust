<div class="hero">
  <p class="eyebrow">LEARNING RUST BY BUILDING LUA</p>
  <h1>从 Lua 解释器开始学习 Rust</h1>
  <p class="hero-copy">不从语法清单开始，而是亲手拆开一个真实解释器：Value、Table、Lexer、AST、Bytecode 与寄存器 VM。</p>
  <div class="hero-actions">
    <a class="primary" href="ch01-start/roadmap.html">查看学习路线</a>
    <a href="ch01-start/setup.html">运行第一个 Lua 程序</a>
  </div>
</div>

## 这不是一本只读不写的书

这本在线书与 [`lua_rust`](https://github.com/YanqingXu/lua_rust) 的真实代码放在同一个仓库里。你看到的关键实现通过 mdBook 的 `include` 机制直接来自 `crates/`，因此教程会跟着代码一起演进。

每一章都会沿着同一条路径展开：

<div class="learning-grid">
  <div><strong>01 · Lua 语义</strong><span>先观察这门动态语言到底承诺了什么。</span></div>
  <div><strong>02 · 实现难点</strong><span>把“看起来理所当然”的行为拆成工程问题。</span></div>
  <div><strong>03 · Rust 工具</strong><span>在 enum、所有权、借用与 Result 中找到对应表达。</span></div>
  <div><strong>04 · 真实源码</strong><span>阅读项目当前使用的实现，而不是孤立的玩具代码。</span></div>
  <div><strong>05 · 设计取舍</strong><span>理解为什么这样写，以及替代方案会付出什么代价。</span></div>
  <div><strong>06 · 动手练习</strong><span>用短实验验证理解，再继续深入。</span></div>
</div>

## 你会做出什么

读完当前主线，你会理解一段 Lua 源码怎样经过完整流水线：

<div class="pipeline">
  <span>Lua source</span><b>→</b><span>Token</span><b>→</b><span>AST</span><b>→</b><span>Bytecode</span><b>→</b><span>Register VM</span><b>→</b><span>Value</span>
</div>

你同时会接触到一组自然出现的 Rust 主题：代数数据类型、模式匹配、切片与生命周期、递归所有权、可变借用、错误传播、泛型容器以及受控的 `unsafe` 边界。

> **项目边界**：`lua_rust` 目前是 Lua 5.1.5 兼容性原型，不是官方发行版替代品。教程会明确区分“已经实现”“正在逼近兼容”和“仍是工程债务”。真实边界比伪装成完美实现更值得学习。

## 适合谁

- 已经写过一点 Rust，希望通过完整项目建立整体感；
- 想学习解释器，却不想从一开始就掉进编译原理术语；
- 熟悉 Lua，想知道动态语义如何落到静态类型系统；
- 在做迁移、兼容性或运行时工程，希望看到可验证的设计过程。

不要求你事先理解 Lua VM。遇到 Rust 语法时，章节会解释它在当前问题中扮演的角色；遇到超出主线的细节，会给出源码入口而不是一次塞满整章。

<div class="checkpoint">
  <strong>准备好了吗？</strong>
  <p>先用 3 分钟查看完整路线，再在本地运行一段 Lua。你会带着一个可观察的结果进入第一章。</p>
  <a href="ch01-start/roadmap.html">开始学习 →</a>
</div>
