# 问题四：借着 record，为什么不能继续修改 Heap？

本实验是 `Heap::mark_one` 的借用边界小模型，不是完整真实 Heap。它只演示：扫描
parent 时读取子边，把拥有型 `Value` 转移到局部 Vec，再修改 gray 队列。没有清扫、
分配、weak、finalizer 或错误恢复，不能把通过结果当作完整 GC 安全证明。

## 先预测，再观察编译器拒绝什么

`before.rs` 借用 `self.records[id]`，`for` 又在每轮继续使用 `record.fields`。
循环内调用 `self.enqueue` 需要另一个 `&mut self`，因此本机 rustc 报 `E0499`。
这份反例故意不能编译，不是 Cargo 自动发现的 target；验收用独立 rustc 子进程检查它，
不会破坏默认 workspace 编译。它与 learner `starter.rs` 的初始方法相同。

从仓库根目录运行：

```powershell
# 2 项通过：确认 before 真正得到 E0499；确认 solution 编译且保留正确子边。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_borrow_reference

# 初始：1 项失败，退出码非零，诊断含 E0499；完成 TODO 后：1 项通过。
cargo test --manifest-path book/code/v14-incremental-gc/Cargo.toml --test gc_borrow_exercises -- --ignored
```

第一条命令加 `-- --nocapture` 可显示实际 rustc JSON 诊断和修复版运行轨迹。若想直接
重放编译器反例（命令退出码应非零），先建立输出目录：

```powershell
New-Item -ItemType Directory -Force target/teaching-v14-callbacks-2026-10-15/borrow/direct | Out-Null
rustc --edition=2021 --error-format=short book/code/v14-incremental-gc/exercises/borrow/before.rs --out-dir target/teaching-v14-callbacks-2026-10-15/borrow/direct
```

测试使用本机 `rustc`（设置 `RUSTC` 时使用该路径），无需下载 trybuild 或其他依赖。
编译器 JSON 诊断、运行输出和独立可执行文件保存在
`target/teaching-v14-callbacks-2026-10-15/borrow/<进程号>-<序号>-<case>/`。
这是本机忽略产物，不随 clone 交付；永久复现入口就是上面的命令和已提交源文件。

## 最小改动与验收

只修改 [starter.rs](starter.rs) 的 `mark_one`。不要改模型、断言或检测器。
先读 [before.rs](before.rs)，尝试将 `record.fields` 收集成 `Vec<&Value>`：其中的引用
仍指向 record，第二次可变借用仍然冲突。需要的是拥有数据的 `Vec<Value>`。

可选参考为 [solution.rs](solution.rs)，核心差异如下：

```diff
- let record = &mut self.records[id];
- record.marked = true;
- for value in &record.fields {
-     self.enqueue(value.clone());
+ let children = {
+     let record = &mut self.records[id];
+     record.marked = true;
+     record.fields.clone()
+ };
+ for value in children {
+     self.enqueue(value);
  }
```

`clone` 在这个模型中复制 `Value` 和数值 ID，不克隆目标对象。块返回拥有型 Vec，
没有从 record 派生的引用逃出，随后 `enqueue` 才重新借用 Heap。Rust 的 NLL 会在
最后一次使用后结束借用；显式块让这个边界容易阅读，不需要 `drop(&mut record)`。
clone 只是分离 Rust 借用，不会把 GC 对象自动加入 roots，保活仍依赖队列及实际扫描。

完成后输出必须精确为：

```text
after parent: gray=[1, 2] marked=[true, false, false, false]
after drain: gray=[] marked=[true, true, true, false]
```

验收先要求两个不同 child 都入队，primitive 不入队、重复边不重复排队；此时只有
parent marked。之后继续扫描 gray，要求两个 child 都 marked、不可达自环仍未标记。
删除整个扫描循环虽然能消除借用错误，也会在行为验收失败。完成 starter 后可再次运行
reference，以确认固定反例仍被拒绝；不要修改 before 来让测试“通过”。

## 对照真实实现

真实 [lua_core/src/lib.rs](../../crates/lua_core/src/lib.rs) 的 `mark_one` 将 `Table`
条目、open/closed upvalue 或 closure upvalues 提取成拥有型工作项，在结束 record 的
临时借用后执行 `self.gray.extend(references)` 和 `self.enqueue_value(value)`。
这里的方法名用 `enqueue` 强调修改队列的第二次
借用。真实实现还有 weak/ephemeron 分类、stale handle 和借用冲突错误，必须分别阅读
真实代码与既有测试，不能从这个四对象模型外推那些合同。
