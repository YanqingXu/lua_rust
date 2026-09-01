# Rust 学习阶梯

<div class="lesson-meta"><span>课程地图</span><span>六层能力</span><span>先做主线，再读专题</span></div>

这不是一张“Rust 语法目录”。每一层都从解释器遇到的真实问题出发，再引入恰好能
解决问题的 Rust 工具。第一次阅读不要跳层：后面的所有权设计会默认你已经能解释前一
层的数据流和失败路径。

## 六层能力

| 层级 | 对应版本 | 解释器问题 | Rust 主线 | 完成标志 |
|---|---|---|---|---|
| 1 · 建立边界 | v0.0 | 源码怎样进入程序，错误怎样离开程序？ | package、模块、`Result`、I/O | 能写并测试一个不 panic 的 CLI |
| 2 · 表示语言 | v0.1–v0.4 | 动态值、Token 和递归语法怎样进入静态类型系统？ | `enum`、`match`、借用、生命周期、`Box` | 能从 Lua 输入手推 Value、Token 和 AST |
| 3 · 编译执行 | v0.5–v0.7 | AST 怎样变成可执行的寄存器指令？ | workspace、位运算、状态机、资源索引 | 能手推一次 codegen 和 VM 寄存器变化 |
| 4 · 动态运行时 | v0.8–v0.12 | Table、函数、闭包和元表怎样共享并改变对象？ | `HashMap`、`Rc`、`RefCell`、身份、动态分发 | 能指出每个对象的 owner 和借用边界 |
| 5 · 管理生命期 | v0.13–v0.15 | 环、回收和可暂停执行怎样保持安全？ | arena、typed handle、图遍历、GC/协程状态机 | 能列出 roots、edges、barrier 和暂停状态 |
| 6 · 形成产品 | v0.16–v1.0 | 标准库、CLI 和兼容结论怎样被验证？ | adapter、进程测试、差分测试、协议治理 | 能区分功能演示、回归证据和兼容证明 |

## 每个 checkpoint 的学习动作

<div class="learning-loop">
  <span>1 · 观察 Lua</span><b>→</b>
  <span>2 · 预测结果</span><b>→</b>
  <span>3 · 建模状态</span><b>→</b>
  <span>4 · 阅读 Rust</span><b>→</b>
  <span>5 · 运行测试</span><b>→</b>
  <span>6 · 修改代码</span>
</div>

如果某一步无法完成，不要直接阅读下一版本。回到本版“代码阅读地图”，找出数据在哪
个函数改变，再用对应测试缩小问题。版本快照是答案，也是调试实验室。

## 第一遍与第二遍

第一遍只追踪主数据流：

```text
source bytes → Token → AST → Instruction/Proto → VM registers → Value
```

第二遍再追踪三条横切线：

- **错误线**：位置怎样从 Lexer、Parser、VM 一直传到 stderr；
- **所有权线**：字节、AST、Proto、Table 和 Closure 分别由谁拥有；
- **证据线**：单元测试、进程测试和双 oracle 分别能证明什么。

## 遇到 Rust 障碍时怎么读

不要只记最终类型。按下面顺序提问：

1. 如果直接拥有这份数据，会发生复制、递归尺寸还是引用环问题？
2. 如果改为引用，引用要活多久，谁保证被引用对象不先释放？
3. 如果需要修改，修改权属于一个 owner、运行时检查的共享 owner，还是 Heap？
4. 如果操作失败，错误属于用户输入、Lua 运行时还是解释器内部损坏？

这些问题分别会自然引出 `Box`、生命周期、`Rc<RefCell<_>>`、typed handle 和
`Result`，也是本课程真正希望建立的 Rust 思考方式。

<div class="checkpoint">
  <strong>开始前检查</strong>
  <p>如果你还不能解释值、共享引用、可变引用和 <code>Result</code> 的基本区别，先完成
  v0.0–v0.1；不需要提前学习 GC、虚拟机或编译原理。</p>
  <a href="../versions/v00-skeleton.html">从 v0.0 开始 →</a>
</div>
