# v0.16：callback 返回 Err 后，为什么下一次合法 yield 也失败？

这题先观察两个相反故障，再写一个共享边界 hook：

1. 进入时 `depth += 1`，callback 后直接 `?`：Err 使函数提前返回，depth 没有减回去，
   后面的合法 yield 也被拒绝。
2. 进入设 1、退出设 0，把嵌套状态当作一个没有前缀的开关：内层结束便清掉外层禁令，
   让本来禁止的 yield 发布 pending payload/frame。

`model.rs` 是同步 Result/深度计数的小模型，不是历史 VM 或完整 coroutine 实现。
模型假定当前已位于可 yield 的 coroutine，不模拟主线程 yield 检测。
`faults.rs` 固定错误设计，`solution.rs` 给出参考；CLI 不调用学习者 starter。
`real.rs` 另用未修改的真实 VM 验证相同 Lua execute 内的可见行为。

## 先预测，再重放

从仓库根目录运行：

```powershell
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example boundary_problem_chain -- error
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example boundary_problem_chain -- nested
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example boundary_problem_chain -- real
```

无参数或 `all` 会运行全部固定对照，未知参数退出 2。CLI 退出 0 不表示学习者已完成
TODO；reference 默认通过也不表示 starter 已修复。

### 问题一：已经得到 Result，为什么还不能用 `?`？

初始 depth=0，边界中 callback 返回 `Ordinary("callback failed")`：

```text
before question-mark: result=Err(Ordinary("callback failed")) depth=1 pending=None frames=[]
  later legal-yield: result=Err(YieldDenied) depth=1 pending=None frames=[]
after capture-result: result=Err(Ordinary("callback failed")) depth=0 pending=None frames=[]
  later legal-yield: result=Err(YieldSignal) depth=0 pending=Some([7]) frames=["yield-frame"]
```

`?` 在这里没有把 Err 变成另一种错误；它跳过了修改长期 VM 字段的收尾语句。Rust 会
正常释放局部拥有的数据，但不会替代码推断“这个 usize 字段必须减回去”。
需要先把整个 Result 保存在局部变量，恢复边界状态，再原样交给调用者。

模型里的 `YieldSignal` 是合法暂停的内部控制信号，不是普通 callback 失败。
真实 VM 也区分可 continuation 的 Yield 与拒绝跨边界产生的 Type 错误；不能把所有
`Err` 都当作需要保存暂停栈的 yield。

### 问题二：内层退出时，为什么不能直接清零？

固定嵌套只发生一次：外层进入 → 内层进入 → 内层普通错误被外层捕获 → 外层尝试 yield。
正确 depth 轨迹为 **0→1→2→1→0**。内层返回以后仍位于外层禁 yield 范围，不能清零：

```text
before boolean: result=Err(YieldSignal) depth=0 pending=Some([99]) frames=["yield-frame"]
  nested: after inner depth=0
after depth: result=Err(YieldDenied) depth=0 pending=None frames=[]
  nested: after inner depth=1
```

这个错误版用 `depth=1/0` 模拟没有保存外层前缀的布尔标记。问题是**无条件清零**，
不能由它推断“任何保存旧状态的布尔实现都不可能支持嵌套”。本 checkpoint 使用计数，
每层保存并恢复自己的前缀，使实际嵌套深度和保护范围都可观察。

拒绝 yield 必须发生在写入 pending values/frames 之前。参考不会在事后清掉一个已经
错误发布的 continuation 来伪装修复；合同要求拒绝时这两个通道始终为空。边界结束后
再发合法 payload 17，最终 pending 只能是 17，不能残留错误版的 99。

## 唯一 TODO：恢复完成之后才把 Result 交出去

只修改 [starter.rs](starter.rs) 的 `invoke_boundary`，不改模型、合同、faults 或真实 VM。
输入是临时 `&mut BoundaryVm` 和一个同步函数指针 callback；callback 可以通过同一 hook
嵌套调用，长期 depth 仍由 VM 拥有。Result 是局部拥有的数据，可以等恢复后再移动出去。

```diff
+ let baseline = vm.depth;
  vm.depth += 1;
- let value = callback(vm)?;
- vm.depth -= 1;
- Ok(value)
+ let result = callback(vm);
+ vm.depth = baseline;
+ result
```

保留可观察的 trace。不要只给 Err 分支补一个 `depth=0`，因为调用前可能已经有三层
禁 yield 前缀，也不要把普通 Err 转成 Ok 来绕过失败路径。答案见 [solution.rs](solution.rs)。

```powershell
# 初始执行 1 项且失败；完成 TODO 后同一项通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test boundary_exercises protected_boundary_restores_depth_on_every_result -- --ignored --exact

# 默认参考执行 1 项且通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test boundary_reference protected_boundary_restores_depth_on_every_result -- --exact
```

共享 [contracts.rs](contracts.rs) 验收 prefix=0/3 下的 Ok、普通 Err、拒绝 yield，检查
深度精确恢复、未发布 pending 状态、退出后合法 yield；再单独验证一次嵌套轨迹。
每次输入有限，模型没有无限递归、真实字节码或宿主异步 callback。

**这里恢复的是正常返回的 Result，不是 panic/abort 安全。** 本实现没有 unwind guard，
callback panic 会绕过恢复语句。不要把一次 Result 收尾合同扩展为所有异常/资源都安全。

## 真实 VM：同一次 execute 内先失败，再合法 yield

```powershell
# 8 项通过：共享合同 1、固定错误有效性 2、真实 VM 5。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test boundary_reference

# 默认 workspace ignored 不表示已完成；初始显式运行 1 项且失败。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test boundary_exercises -- --ignored
```

五个真实 probe 分别覆盖：

- `pcall` 捕获普通错误、拒绝 callback yield；
- `xpcall` 普通错误由 handler 返回数字 17，另外拒绝目标与 handler 的 yield；
- `table.sort` 的 comparator 普通失败与 yield 尝试，由外层 pcall 捕获；
- callable table 的 `__call` 成功值，以及 `__call` 内的 yield 尝试；
- 外层 pcall 中的内层普通错误已被捕获，直接 yield 仍受外层保护。

每个 case 都在**同一次 `execute_multi`** 创建 coroutine，在这些错误之后执行合法
`coroutine.yield(checked,41,nil)`，随后 resume 输入 42，最终 return true,99 并 Dead。
预期完整 Vec 都是：

```text
[Boolean(true), Boolean(true), Number(41.0), Nil, Boolean(true), Boolean(true), Number(99.0), Boolean(true)]
```

依次表示第一轮 resume 成功、case 检查成功、合法 yield 的 41/nil、第二轮 resume 成功、
参数方向正确、返回 99、最终 Dead。yield payload 中的 nil 不会被改成错误字符串或丢失；forbidden
callback 的旧 payload 也不能替代 41。每个 probe 只有一个 execute，没有用新的
execute/VM 清空状态来掩盖恢复缺陷；真实 probe 不读取私有 depth/pending 字段。

## 与真实源码对应，边界在哪里？

真实 [lua_vm/src/lib.rs](../../crates/lua_vm/src/lib.rs) 的 `invoke_yield_forbidden`
使用 `+=1 → let result=invoke_callable(...) → -=1 → result`。参考模型显式保存 baseline，
是为了把“恢复调用前前缀”写成合同；不宣称真实源码已有同名 baseline 变量。
在配对正常返回时，两者都保留外层深度。

`invoke_callable` 先 `resolve_callable`；如果经过 `__call`，还会增加一层 forbidden
深度，并在 invoke 的 Result 返回后减回去。`CoroutineYield` 在 depth 非零时先返回
Type 错误，尚未写 `yield_values`；Call/TailCall 仅在真正 Yield 控制信号时保存帧。
因此“被禁止的 yield”与“已暂停后等待 resume”的数据路径不同。

pcall/xpcall/comparator/`__call` 都实际调用这些路径。当前报错文案包含
`metamethod or finalizer boundary`，即使触发入口是 pcall/sort 也如此；probe 只检查
其当前 `boundary` 文本与之后的控制流，不冒充已经改进了错误诊断。

**depth 恢复不能证明所有 VM 资源都已恢复。** `invoke` 里的 CoroutineWrapper 分支
仍有 `resume_coroutine(...)?` 位于 `temporary_roots.truncate(root_base)` 之前；如果
该 resume 本身返回 Err，可能提前越过 roots 收尾。本题不修复或证明这个支路，也不
验证任意 wrapper/GC/宿主重入组合。sort callback 资源保活、完整 Yield 协议、panic
处理、增量 GC phase 与任意 native 栈深同样不在本包证明范围。

理解题：为什么 Result 本身可以保存，而长期 depth 需要显式恢复？
实现题：修复唯一 hook，并运行显式 learner 单题验收。
调试题：内层已经返回 Err，外层为何仍要保持 depth=1？非法 pending99 是何时发布的？
完成时应能解释“depth 泄漏导致过度拒绝”与“清零导致过早放行”这两个相反故障。

本机证据写到 `target/teaching-v16-callbacks-2026-10-15/boundary/`；忽略产物不随 clone
提供，永久入口是本 README、starter/solution/contracts 和固定 CLI 源文件。
