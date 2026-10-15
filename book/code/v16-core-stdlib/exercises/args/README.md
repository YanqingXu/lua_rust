# v0.16：参数转换为什么需要一个共享入口？

先预测两个不一致：一个 Rust builtin 只接受 `Value::Number`，另一个允许数字字符串，
它们遇到同一个 `" 42.5 "` 会怎样？如果字符串函数先转成 Rust UTF-8 文本再处理，
Lua 字节 `[97,0,255,122]` 又会变成什么？

本包用两个刻意写坏的独立 adapter 重放问题，不把它们冒充历史 VM 实现。
学习者只修改 [starter.rs](starter.rs) 的 number/bytes 两个函数；参考解直接调用当前
真实 `lua_stdlib::Args`。测试同时使用真实 `Value`、`HostError` 和 VM，没有另造 Lua
数字词法、对象模型或垃圾回收器。

## 先重放，再修改

从仓库根目录运行：

```powershell
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example args_problem_chain -- number
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example args_problem_chain -- bytes
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example args_problem_chain -- real
```

省略参数或用 `all` 一次重放全部；未知参数退出 2。CLI 固定运行 faults/solution，
不读取 starter。退出 0 只表示重放完成；默认 reference 通过也不表示已经完成 TODO。

| 输入 | 固定错误版 | 共享 Args |
| --- | --- | --- |
| 第二个参数 `" 42.5 "` | number required，无索引 | `Ok(42.5)` |
| 缺少第一个参数 | number required，无索引 | argument=Some(1)，value expected |
| 显式 nil | 与缺参混淆 | argument=Some(1)，number expected, got nil |
| 第二个参数 true | 与其他失败混淆 | argument=Some(2)，number expected, got boolean |
| bytes `[97,0,255,122]` | `[97,0,239,191,189,122]` | 原四字节不变 |
| Number 12.5 → bytes | string required | `[49,50,46,53]`，即 `12.5` |

错误文本化产生的 239/191/189 是 UTF-8 replacement character 的三个字节，已经改变
输入。NUL 和非 UTF-8 byte 都可以是 Lua 字符串内容，不应先要求它是 Rust `String`。

## TODO 1：统一 number 转换与错误分类

只修改 `starter.rs::number`。函数借用调用者的 `&[Value]`，使用从 0 开始的 index，
返回不借用输入的 f64 或拥有型 HostError。需要共享的规则为：

- Number 直接返回其 f64；
- String 按当前 checkpoint 的 number 解析规则转换；
- 缺参先由 required 产生 `value expected`；
- nil、boolean 等已提供的错误类型有 `number expected, got TYPE`；
- 字符串已提供但解析失败，错误为 `number expected`。

```diff
- match values.get(index) {
-     Some(Value::Number(number)) => Ok(*number),
-     _ => Err(HostError::message("number required")),
- }
+ Args::new(values).number(index)
```

记得从 `lua_stdlib` 引入 `Args`；不要修改 contracts 或 Args 本身。参数切片继续属于
调用者，adapter 没有克隆整个 `Vec<Value>`。需要答案时看 [solution.rs](solution.rs)。

```powershell
# 初始执行 1 项且失败；完成 TODO 1 后通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_exercises number_coercion_and_errors_are_shared -- --ignored --exact

# 默认参考执行 1 项且通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_reference number_coercion_and_errors_are_shared -- --exact
```

合同检查数字字符串、负小数、指数、缺参、nil、boolean、第二/第三参数位置，以及空串、
无效文本、非 UTF-8 输入和 `0x10` 的当前结果。本快照的 number 字符串路径是
**UTF-8 → trim → Rust f64 parse**；`0x10` 在这个路径被拒绝。这里不是完整 Lua lexer，
不是官方 Lua 全部数字词法或全部数值格式的兼容性证明，也不把 `tonumber` 的独立 base
分支当成 `Args::number` 的同一入口。

## TODO 2：借用输入，为什么 bytes 仍要返回拥有型 Vec？

只修改 `starter.rs::bytes`，让 String 保留原字节、Number 使用当前快照的数字格式输出
字节，同时复用 required 和索引错误：

```diff
- match values.get(index) {
-     Some(Value::String(bytes)) => Ok(String::from_utf8_lossy(bytes.as_bytes())
-         .into_owned().into_bytes()),
-     _ => Err(HostError::message("string required")),
- }
+ Args::new(values).bytes(index)
```

这里的返回类型是 `Vec<u8>`：对 String 复制原字节，对 Number 必须生成新字节。
调用者可以反转或修改这个 Vec，也可以在原参数切片释放后继续拥有它。它不是指向
临时 `number.to_string()` 的悬空切片；复制 Vec 不会改变输入 ByteString。

```powershell
# 初始执行 1 项且失败；完成 TODO 2 后通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_exercises bytes_remain_owned_and_binary_safe -- --ignored --exact

# 默认参考执行 1 项且通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_reference bytes_remain_owned_and_binary_safe -- --exact
```

验收必须看到原样 `[97,0,255,122]`，不能只确认输出仍可显示。它还修改输出 Vec 后
重新读取输入，要求输入不变；在局部参数数组离开作用域后继续检查返回 bytes。
bytes 也区分缺参、nil、boolean，并以第二/第三位置验证索引，而非一律写 `#1`。

## Args 的 `'a` 与 HostError 的所有权

真实 [lua_stdlib/src/lib.rs](../../crates/lua_stdlib/src/lib.rs) 的 `Args<'a>` 只存
`&'a [Value]`。复制 Args 只复制视图，不复制 Value。`get`/`required` 返回 `&'a Value`，
所以可以在这个 Args 视图本身用完后继续借用原 Value，但不能超过参数存储的生命期。
`number` 返回 f64，`bytes` 返回新 Vec，`HostError.message` 是拥有型 String，都没有
把这个 `'a` 带到返回值里。

[real.rs](real.rs) 的 `borrowed_value` 接收一个 Args 视图，返回原切片里的引用，
用 `std::ptr::eq` 验证没有复制 Value；`owned_bytes_after_input_drops` 则在函数内创建
参数，返回仍有效的 bytes。生命周期由 Rust 类型系统约束，pointer 比较仅帮助观察别名，
不是运行时补救悬空引用的机制。该路径未使用 unsafe。

required 与 optional 也不能混为一谈：`required(0)` 对缺参报错，但对已提供 nil 返回
`&Value::Nil`；随后 `number` 才将 nil 分类为错误类型。`optional_integer(0,12)` 明确
选择“缺参或 nil 均用 12”，而 false 仍报错。不能把所有检查统一成一个 `unwrap_or`。

## 谁补上函数名，谁决定 VM 错误种类？

`Args` 输入 index 从 0 开始，`HostError::argument(index, ...)` 保存 `index+1`，
显示为 `bad argument #N (reason)`。HostError 本身不包含函数名；真实 VM adapter
通过 `host_error(function.name(), ...)` 转为 `VmErrorKind::Type` 并追加函数名：

```text
bad argument #1 (value expected) to 'math.sqrt'
bad argument #1 (number expected, got nil) to 'math.sqrt'
bad argument #2 (number expected, got boolean) to 'math.pow'
bad argument #1 (string expected, got boolean) to 'string.len'
```

这就是当前 checkpoint 的精确格式，不能改写成另一个 Lua 实现的标点顺序再声称逐字
匹配。真实 VM 对照还执行数学/字符串函数，得到 9、2、3、3、255，证明选定调用路径
确实使用共享转换，而非只在测试中手工格式化一条 HostError。

## 完整验收和边界

```powershell
# 9 项通过：2 共享合同、2 固定故障、3 真实 Args、2 真实 VM。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_reference

# 初始两题失败，退出码非零；完成后两题均通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test args_exercises -- --ignored
```

默认 workspace 编译 starter，但两题为 ignored；默认通过不表示练习完成。两个 hook
没有相互依赖，可以独立修复。不要改断言、faults、核心实现或默认 ignored 标记。

本包只验收选定的 Args number/bytes、required/optional 边界及 math/string 调用路径。
不覆盖全部 stdlib 参数规则、全部 numeric formatting、堆分配/GC roots、C ABI、受保护
调用清理或其他宿主重入边界。返回 Vec 的分配成本也没有在这里做性能证明。

理解题：Args 是 Copy，为什么参数 Value 仍没有被复制？缺参为什么不同于显式 nil？
实现题：完成两个 adapter，并分别运行其显式单题命令。
调试题：输入 byte 255 变成三字节 239/191/189，应该先检查 UTF-8 转换还是 string.len？
完成后应能说明借用切片、返回值所有权、参数索引与 VM 函数上下文分别属于哪一层。

本机证据独立写到 `target/teaching-v16-2026-10-15/args/`，忽略产物不随 clone 提供；
永久复现入口是本 README、contracts、starter/solution 与 CLI 源文件。
