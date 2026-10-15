# v0.16 · 核心标准库

<div class="lesson-meta"><span>参数规则一致 → 目录驱动安装 → 唯一全局环境</span><span>5-crate workspace</span><span>checkpoint v0.16</span></div>

<a id="learning-task-card"></a>
## 学习任务卡

<div class="task-card">
  <strong>从三个可见故障推导 Rust builtin 的边界</strong>
  <p>前置：v0.01 字节字符串、v0.08 Table、v0.14 Heap roots、v0.15 VM 调用。
  本次依次解决：同类参数被不同函数解释；函数写好了却找不到；裸 global 与 _G 各改
  一份数据。产出是四个独立 TODO、错误/修复对照、三幅图和真实 Heap/VM 验收。
  新知识是借用参数切片、拥有的返回值、结构化错误、crate 依赖方向和共享对象身份。</p>
</div>

v0.15 已能调用少量 Rust builtin。v0.16 加入数十个入口后，复制取参和安装代码会把
同一规则散落到不同位置。本章先观察这种写法的失败，再引入 `Args`、catalog 与唯一
global environment。`lua_stdlib` 管声明和参数规则，VM 保管 Heap 与执行状态。

三个练习包是**人为构造的局部错误设计**，不是声称历史版本曾经出现这些 bug，也不会
修改真实 VM 来制造故障。`faults.rs` 固定错误行为，`solution.rs` 给出参考，`real.rs`
另接真实快照。CLI 不读取 `starter.rs`，重放成功不表示学习者已经完成练习。

<a id="learning-predict"></a>
## 先预测再运行

先写下四组预期：`math.abs('-9')` 与 `math.sqrt('9')`；`math.sqrt()` 与
`math.sqrt(nil)`；`string.char(65,0,255)` 的字节；`answer=42` 后的 `_G.answer`。
最后问：函数体存在，是否足以让 `math.sqrt` 不为 nil？

把预测写完，再读这个真实 Lua 探针。它还检查错误原因不同、双向读写、删除及 `_G` 重绑：

```lua
{{#include ../../code/v16-core-stdlib/examples/builtin_contracts.lua}}
```

以下命令均从仓库根目录运行。省略 CLI 参数或用 `all` 会执行该包全部固定对照：

```powershell
$stdManifest = 'book/code/v16-core-stdlib/Cargo.toml'
cargo run --quiet --manifest-path $stdManifest -- book/code/v16-core-stdlib/examples/builtin_contracts.lua
cargo run --quiet --manifest-path $stdManifest --example args_problem_chain
cargo run --quiet --manifest-path $stdManifest --example catalog_problem_chain
cargo run --quiet --manifest-path $stdManifest --example globals_problem_chain
```

真实快照接受上述数字字符串，字节值仍为 `[65,0,255]`；初始 `_G` 指回环境表，所以
裸写入立即能从 `_G` 读到。缺参和显式 nil 都失败，但失败原因不同。下面逐步对照。
第一条 Lua 探针应返回 `type=boolean value=true`；三个 CLI 则逐项显示故障与参考结果。

<a id="learning-code-map"></a>
## 代码阅读地图

1. [参数包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v16-core-stdlib/exercises/args/README.md)：固定取参错误 → `Args<'a>` → `HostError` → VM 附加函数名；
2. [注册包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v16-core-stdlib/exercises/catalog/README.md)：漏安装/错 ID → `FunctionSpec/LibrarySpec` → `Vm::new` → `invoke_native`；
3. [全局环境包](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v16-core-stdlib/exercises/globals/README.md)：两份状态 → 一个 Heap table → `GETGLOBAL/SETGLOBAL` → `gc_roots`；
4. [真实 stdlib](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v16-core-stdlib/crates/lua_stdlib/src/lib.rs)：目录、decoder 与错误数据；
5. [真实 VM](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v16-core-stdlib/crates/lua_vm/src/lib.rs)：初始化、Heap mutation、builtin 分派与错误转换；
6. [既有标准库合同](https://github.com/YanqingXu/lua_rust/blob/main/book/code/v16-core-stdlib/tests/stdlib.rs)：公开库成功值、coercion、错误分类和可选 oracle 对照。

## 问题一：同一类参数，为什么一个函数接受、另一个拒绝？

### 每个 builtin 自己取参，就可能各写一套规则

假设 `sqrt` 已接受数字字符串，而新增 `abs` 只匹配 `Value::Number`。`abs('-9')`
便在运算前失败，尽管 `sqrt('9')` 能返回 3。先运行固定反例，观察拒绝发生在哪一步：

```powershell
cargo run --quiet --manifest-path $stdManifest --example args_problem_chain -- number
```

CLI 把差异隔离在通用取参函数：输入同一字符串 `' 42.5 '`，strict-number 故障版拒绝，
共享 `Args::number` 返回 42.5。上面的 abs/sqrt 是引入情景，真实 Lua 探针验证两者均已修好。

只认 Number 的起点可编译，也能处理数值，但它丢失了数字字符串转换和参数位置：

```rust
{{#include ../../code/v16-core-stdlib/exercises/args/starter.rs:args_number_starter}}
```

第一步不改数学运算，只把取参交给共享 decoder，并从 `lua_stdlib` 导入 `Args`：

```diff
- match values.get(index) {
-     Some(Value::Number(number)) => Ok(*number),
-     _ => Err(HostError::message("number required")),
- }
+ Args::new(values).number(index)
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/args/solution.rs:args_number_solution}}
```

`Args<'a>` 只借用调用者已有的 `&'a [Value]`。`get/required` 返回同一切片内的
`&'a Value`，没有复制 VM 或长期保存参数；`number` 则在局部借用期间解出拥有的
`f64`。生命周期 `'a` 把返回引用的有效期约束在参数数据之内，不负责 GC 保活。

```rust
{{#include ../../code/v16-core-stdlib/exercises/args/real.rs:args_borrowed_view}}
```

| 输入情况 | 真实 decoder 的步骤 | 可见结果 |
| --- | --- | --- |
| number | 直接取值 | 对应 `f64` |
| 数字字符串 | UTF-8 检查、trim、Rust `f64` parse | 如 `' 9 '` 得到 9 |
| 缺参 | `required` 发现 `None` | `value expected` |
| 显式 nil | 参数存在，但类型不符 | `number expected, got nil` |
| 非数字字符串或非法 UTF-8 | 数值解析失败 | `number expected` |

`HostError::argument(index, ...)` 把内部零基下标转为一基参数编号；错误仍是数据，
尚无函数名。VM 的 `host_error(function, error, proto, pc)` 再添加函数名与源位置，
将这一类错误标为 `VmErrorKind::Type`。不能把任意 Heap 或 Lua callback 错误都
重新包装成 HostError；那些错误应保留自己的来源。回调边界留待后续微课。

![共享参数规则、借用输入、拥有的输出和结构化错误的流向。](../images/v16-argument-contract.svg)

### string 参数为什么不能直接变成 Rust String？

把字节经过 lossy UTF-8 文本转换，再转回 bytes，会把 `0xff` 改写为替换字符的编码。
这次“转换成功”实际上已经改变数据。固定错误版同时漏掉了 number → 字节文本的规则：

```powershell
cargo run --quiet --manifest-path $stdManifest --example args_problem_chain -- bytes
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/args/starter.rs:args_bytes_starter}}
```

```diff
- match values.get(index) {
-     Some(Value::String(bytes)) => Ok(String::from_utf8_lossy(bytes.as_bytes())
-         .into_owned().into_bytes()),
-     _ => Err(HostError::message("string required")),
- }
+ Args::new(values).bytes(index)
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/args/solution.rs:args_bytes_solution}}
```

这里返回 `Vec<u8>`，即拥有的副本；源参数销毁后，这个 Vec 仍可使用。这样 builtin
后续处理不用继续借着参数容器。复制字节有成本，但保留 NUL/0xff 是语义要求，
不能用“文本看起来一样”代替逐字节验收。

```rust
{{#include ../../code/v16-core-stdlib/exercises/args/real.rs:args_owned_bytes}}
```

```powershell
cargo test --manifest-path $stdManifest --test args_reference real_args_borrow_values_while_owned_bytes_can_escape_input_lifetime -- --exact
```

该单题用指针相等帮助观察 `required` 返回原 Value 的别名，再验证源参数离开作用域后
拥有的 bytes 仍可读取。引用有效期本身由 Rust 类型系统保证，不靠指针比较补救。

**完成条件：** 两个 hook 分别通过数字字符串/缺参/类型/编号合同和 bytes/number
转换合同；真实 VM 还要验证错误分类、函数名以及第 2 个参数的编号。两个 TODO 独立，
只修一个即可运行对应验收，完整命令见参数包 README。

本版边界：数值解析使用 UTF-8 + Rust `f64`，没有证明全部 Lua 5.1 数值词法相同；
`optional_integer` 把缺参与 nil 都视为默认值，不能推广成所有 decoder 都这样做。
`Args::table` 只检查 Value 变体并取 typed handle，句柄有效性由后续 Heap API 验证。

接下来，参数规则统一了，却仍可能根本调用不到写好的函数。

## 问题二：函数体已经实现，为什么 Lua 查到的还是 nil？

### 分派分支与安装目录是两件事

在固定错误设计里，VM adapter 已能处理 `MathFunction::Sqrt`，安装器却在完整目录中
跳过了 `sqrt`，其余 67 个绑定仍存在。Lua 的 `math.sqrt` 查表得到 nil，尚未进入运算：

```powershell
cargo run --quiet --manifest-path $stdManifest --example catalog_problem_chain -- missing
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/catalog/starter.rs:catalog_install_starter}}
```

把公开名字与稳定身份放进 `FunctionSpec`，按库分组为 `LibrarySpec`，让初始化遍历
这些声明。展开 `starter` 委托的固定 `omit_sqrt`，最小修正就是删掉对目录的例外过滤：

```diff
  for &spec in library.functions {
-     if library.name == b"math" && spec.name == b"sqrt" {
-         continue;
-     }
      installed.bind(table, spec)?;
  }
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/catalog/solution.rs:catalog_install_solution}}
```

目录中的名字是静态字节切片，spec 的身份是可复制的 `NativeFunction` enum。
复制 ID 不会复制函数执行状态，也不是捕获 VM 的 Rust closure。它可以放进 Value、
比较或用作 table key；VM 收到 ID 后再穷尽分派。目录自身不拥有函数体。

![从 core 身份到 stdlib 声明，再由 VM 安装并执行；查表和执行分别验收。](../images/v16-catalog-installation.svg)

### “有一个 function”为什么仍不足以验收？

把 `sqrt` 错装成 `abs` 时，类型仍然是 function，调用也能返回 number；输入 81
却得到 81，而非 9。因此需要独立的“名字 → ID”期望，再验证实际调用结果：

```powershell
cargo run --quiet --manifest-path $stdManifest --example catalog_problem_chain -- identity
cargo run --quiet --manifest-path $stdManifest --example catalog_problem_chain -- duplicates
cargo test --manifest-path $stdManifest --test catalog_reference catalog_installs_the_independent_public_contract -- --exact
```

合同中的期望不能临时从待测 catalog 生成，否则 catalog 自己漏掉 sqrt 时，两边会一起
缺项。练习用独立公开清单核对目录和安装结果，再用遗漏、错 ID 的固定变体证明合同能
发现问题。同库重复名字应被教学 preflight 拒绝；不同库的同名函数可以共存。

安装实验使用独立 Heap。`installed_call` 先从这张表读出稳定 ID，将固定探针编译结果
中唯一的数值 0 常量替换为该 NativeFunction，再交给新建真实 VM 执行。这证明“装进去
的身份确实调用了预期函数”；它不表示实验 Heap 已被装入真实 VM。另一个直接 Lua
探针检查真实 `Vm::new` 的公开入口。下面每个调用使用独立固定期望：

```rust
{{#include ../../code/v16-core-stdlib/exercises/catalog/real.rs:catalog_real_calls}}
```

**源码对应与边界：** 真实 `Vm::new` 已遍历 `BASE_FUNCTIONS/LIBRARIES` 安装公开库，
另装 math 常量和 string primitive 的 `__index`；`gc_*` 等教学诊断仍单独安装。
练习的重复名 preflight 是局部保护，真实初始化没有相同的动态检查。穷尽 enum 分派
能帮助发现遗漏分支，却不会自动证明 catalog 没有漏声明或连错 ID。

### 为什么拆 crate，到这里才有直接收益？

三个职责现已能够从失败位置区分：stdlib 声明公开目录和纯参数规则，core 定义可跨层
传递的 Value/ID，VM 拥有 Heap、调用栈和实际执行。因此依赖是 `stdlib → core`，
`VM → stdlib + core`；app 再把 compiler 与 VM 组合起来。stdlib 不需要反向依赖 VM。

新增一个 builtin 时仍要修改 ID、catalog、VM dispatch；集中目录解决重复安装清单，
并不消灭所有改动点。收益通过“漏安装被发现、错 ID 被发现、调用结果正确”来观察。

**完成条件：** 修复 installer 后，独立公开合同通过，固定故障仍被识别；真实 VM 的
对应函数可调用。只把 `sqrt` 塞回一个特例、只检查 function 类型，都不是目录驱动安装。

函数可以找到了，但库表到底安装进了哪一个全局环境？

## 问题三：为什么 answer 与 _G.answer 会各自保留一个值？

### 给 Rust globals 复制一个 _G 快照，会产生两个写入口

v0.15 使用 Rust `HashMap<ByteString, Value>` 保存 global。若沿用它，再为了暴露 `_G`
而复制一个 Heap table，裸 `answer` 与 `_G.answer` 就落在不同位置。固定反例先复制
快照，再从两边写入；第二次写入不会自动通知另一边：

```powershell
cargo run --quiet --manifest-path $stdManifest --example globals_problem_chain
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/globals/starter.rs:globals_install_starter}}
```

展开错误构造器，就能看到快照复制后出现两份长期存储：

```rust
{{#include ../../code/v16-core-stdlib/exercises/globals/model.rs:globals_snapshot_storage}}
```

这题只替换环境初始化 hook。驱动器已提供两种设计的读写路径，健康路径始终从同一
环境句柄读取和修改。真实 v0.15 只有 globals map，尚未安装 `_G`；双存储是本节
人为设计的失败迁移。真实 v0.15 → v0.16 的变化概括如下：

```diff
- globals: HashMap<ByteString, Value>
+ global_env: TableRef
+ GETGLOBAL / SETGLOBAL → heap.table_get / table_set(global_env, ...)
+ global_env["_G"] = Value::Table(global_env)
+ gc_roots 显式加入 Value::Table(global_env)
```

```rust
{{#include ../../code/v16-core-stdlib/exercises/globals/solution.rs:globals_install_solution}}
```

这里“唯一”指表的身份相同。Rust 保存/复制的 `TableRef` 是句柄，不是表内容副本。
table 的存储由 Heap 管，VM 保管环境句柄；把句柄放进 `gc_roots` 后，collector 才从
这个环境继续追踪字段。单有 Rust owner 或 `_G` 自环都不能代替根枚举。

```rust
{{#include ../../code/v16-core-stdlib/exercises/globals/model.rs:globals_environment_roots}}
```

![从两份 globals 到同一 Heap 环境；替换 _G 字段不会替换 VM 的环境身份。](../images/v16-global-environment.svg)

真实 VM 的两条指令都使用 `self.global_env`，不需要维护同步副本。注意 `SETGLOBAL`
通过 Heap API 写入，以保留 nil 删除与 write barrier；不能直接绕过 Heap 修改内部 map。

```rust
{{#include ../../code/v16-core-stdlib/crates/lua_vm/src/lib.rs:globals}}
```

### 沿一条验收规格理解“同一份状态”

```powershell
cargo test --manifest-path $stdManifest --test globals_reference single_environment_is_shared_by_globals_and_g -- --exact
```

这条共享合同让同一个初始化 hook 接受以下观察，而非只比较两个句柄的显示文本：

1. 裸 global 写入后，从 `_G` 路径读到新值；从 `_G` 写入后，裸读也改变。
2. 通过任一路径写 nil 后，另一边精确读到 `Value::Nil`，没有残留快照值。
3. 初始 `_G` 与环境是同一张表，`_G._G` 也回到它；图的自环可被遍历终止。

参考包再验证跨 GC 的图：真实 VM 分多次 `execute` 建立全局对象、在宿主调用收集、
重新执行读取。这样第一次调用的局部寄存器不会替根枚举遮掩问题。独立 Heap 另撤掉
所有环境根，验证自环可以被回收；Rust 中仍留着句柄不妨碍它变 stale。

### _G 可以改写，环境身份由谁保持？

先保存 `local env = _G`，再执行 `_G = { answer = 99 }`。这是在原环境 E 的 `_G`
字段上写入新表 R；`Vm.global_env` 仍指 E。所以裸 `answer` 仍访问 E，`_G.answer`
此时访问 R。`_G` 是普通可变字段，不能将“初始化时指回自身”教成永久不可变条件。

**完成条件：** 同步读写和删除成立，宿主 GC 后对象可访问，替换 `_G` 后裸 global
仍来自原环境。真实 VM 与独立 Heap 分别验证对应观察，不从一次成功运行外推所有 GC
路径安全。v0.15 已记录的活动 varargs/tail-call 漏根属于另一个工程问题，本包未修复。

## 其余标准库能力：保留 checkpoint 的范围

本快照还实现以下能力；本轮三个问题不展开受保护回调和排序过程的所有权协议：

| 库 | 本版能力与边界 |
| --- | --- |
| Base | 19 个入口，包括 assert、pcall/xpcall、print、raw 操作、select/unpack、类型/转换和 collectgarbage |
| Math | 28 个函数与 pi/huge；固定 LCG 让 seed 测试可重复，不承诺与平台 C rand 数列相同 |
| String | 10 个 byte-string 入口；find 暂为 plain search；format 只验证 `%s/%d/%i/%f/%q/%%` |
| Table | concat/insert/maxn/remove/sort；移动通过 Heap mutation；sort 为教学 insertion sort |
| Coroutine | 六入口纳入 catalog，保留 v0.15 scheduler 与禁止 yield 边界 |

`pcall/xpcall` 成功返回 `true, results…`；pcall 失败返回 `false, message`，xpcall 先把
错误交给 handler，再返回 `false, handler 的首个结果`（无结果时为 nil）；handler 失败
则返回错误文本。其 callback 与 table comparator 跨不可 continuation 的边界时禁止 yield。具体的进入、
失败、恢复与资源保活应作为下一组问题单独验证，不能由参数包的 HostError 测试代替。

原 `examples/stdlib_diff.lua` 及差分测试仍保留：有 `lua` 可执行程序时启动外部对照，
找不到则跳过该外部进程。因此“workspace 全通过”不等于本机执行过官方 oracle；应记录
实际可用版本和是否执行。本轮新合同不依赖外部 oracle 即可检查固定期望。

## 运行与验收

```powershell
cargo run --quiet --manifest-path $stdManifest -- book/code/v16-core-stdlib/examples/demo.lua
cargo test --manifest-path $stdManifest --test args_reference
cargo test --manifest-path $stdManifest --test catalog_reference
cargo test --manifest-path $stdManifest --test globals_reference
cargo fmt --manifest-path $stdManifest --all -- --check
cargo clippy --manifest-path $stdManifest --workspace --all-targets -- -D warnings
cargo test --manifest-path $stdManifest --workspace
cargo test --manifest-path $stdManifest --workspace --release
```

demo 应返回 `type=number value=1616`。本包加入 24 项参考：args 9、catalog 8、globals 7；
与原有 109 项合计 **133 项通过、4 项 learner ignored**。默认参考检查修复方案、
固定故障及真实快照对照；不能把默认通过当作练习完成。显式运行：

```powershell
cargo test --manifest-path $stdManifest --test args_exercises -- --ignored
cargo test --manifest-path $stdManifest --test catalog_exercises -- --ignored
cargo test --manifest-path $stdManifest --test globals_exercises -- --ignored
```

三个命令初始分别有 2、1、1 个失败。只改各包 `starter.rs` 指定 hook，完成后相应
命令全部通过；不要改断言、固定 faults 或参考入口。逐题精确命令与预期见各包 README。
维护者验证还要完成“初始失败 → 临时套参考解通过 → 恢复原字节后再次失败”，确认练习
确实调用 starter，也确认仓库交付的是待完成版本。

站点验收继续使用：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/check_book_teaching.ps1
mdbook build book
node tools/check_book_links.mjs target/mdbook .
```

## 本版本暂不支持

- 完整 Lua 数值词法、string pattern、完整 string.format；string.dump 留到 binary chunk。
- `dofile/load/loadfile/loadstring` 与 io/os/package/debug 留到 v0.17；function environment 留待兼容收口。
- `collectgarbage('count')` 暂按对象数；setpause/setstepmul 尚不改变阈值。
- 非 ASCII 大小写的 locale 行为、与 C rand 相同的随机序列、生产排序性能。

<a id="learning-exercises"></a>
## 章末练习

<div class="exercise understanding">
  <strong>理解题：找出每一份数据的拥有者</strong>
  <p>沿参数图说明 Args 借用了什么、bytes 返回谁的 Vec、HostError 何时获得函数名；
  沿注册图说明名字、ID、函数体和 Heap 分别由哪层管理。以参数包 real 对照命令验收，
  成功条件是能解释输入销毁后为何返回 bytes 仍可使用，且不把它当作 GC root。</p>
</div>

<div class="exercise implementation">
  <strong>实现题：完成四个独立 hook</strong>
  <p>修改 args 的 number/bytes、catalog 的 installer、globals 的 initializer，范围只限
  三个 starter.rs。运行上面的三个 --ignored 命令；四项均通过才完成。对照代码 diff
  说明每一步解决哪条失败观察，不以删除反例或修改合同取得通过。</p>
</div>

<div class="exercise debugging">
  <strong>调试题：存在性和同一性是两种检查</strong>
  <p>运行 catalog_problem_chain -- identity，定位为什么 function 类型正确仍算失败；
  再运行 globals_problem_chain，对照替换 _G 前后的裸读取。成功条件是分别指出错 ID
  与换字段的差别，解释为何后者没有替换 VM 的环境句柄。无需修改核心源码。</p>
</div>

<a id="learning-checkpoint"></a>
## 完成检查点

- 用同一组输入说明独立解析为什么分叉，统一 decoder 后哪些成功/错误观察改变了。
- 能解释借用切片与拥有的 bytes 输出，并保留缺参、nil、错误类型之间的区别。
- 能让遗漏/错 ID/重复名变体被验收发现；说明目录、分派和诊断入口各自的范围。
- 能证明双向 global 读写、nil 删除、跨 GC 存活与 `_G` 可替换，并画出实际根路径。
- 四个独立 TODO 通过；能区分固定模型、真实快照验证与尚未验证的工程边界。

## 从 v0.15 迁移到哪里，下一步又会遇到什么？

保留 Value、Heap、调用与 coroutine 协议；新增 `lua_stdlib` 的 catalog/Args/HostError，
让 VM 遍历声明安装公开库。global 从 Rust map 改成 Heap table，指令读写与 roots 同时
迁移；仅安装 `_G` 快照不能完成这次迁移。标准库函数体继续位于 VM adapter。

下一组 v0.16 问题可从“普通 Rust builtin 能返回，调用 Lua callback 后为什么还要
恢复边界状态”开始，讲 pcall/xpcall、sort comparator 与禁止 yield。v0.17 再扩展到
文件 loader、io/os/package/debug 和 require 的缓存/循环合同。

可回看 [Value 与字节字符串](v01-value.md)、[Table](v08-table.md)、
[增量 GC](v14-incremental-gc.md) 与 [Coroutine](v15-coroutine.md)，追溯这些规则的来源。
