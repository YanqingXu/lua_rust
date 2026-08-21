# v0.16 · 核心标准库

<div class="lesson-meta"><span>核心增量：统一的 Rust builtin 合同</span><span>5-crate workspace</span><span>checkpoint v0.16</span></div>

v0.15 已经能调用 Rust 写成的函数，但注册、取参和错误文案散落在 VM 初始化与 dispatch 中。本版增加一个核心能力：**用独立 `lua_stdlib` crate 声明公开 API，并让所有 builtin 共享同一套参数 coercion、返回值和 Lua 错误协议**。随后在这个边界上实现 base、math、string、table 和 coroutine 五组核心库。

## 为什么现在才拆 `lua_stdlib`

早期只有 `next`、metatable helper 和 GC 诊断入口，额外 crate 只会增加跳转成本。v0.16 一次加入数十个函数，注册目录、参数规则与 Heap mutation 已经是三个不同职责：

```text
lua_core <── lua_stdlib
    ▲              ▲
    │              │ catalog / Args / HostError
    └──── lua_vm ──┘
              ▲
              │
           lua_app
```

`lua_stdlib` 依赖 `lua_core` 以使用 `Value` 和 builtin ID；`lua_vm` 同时依赖两者，并继续独占 Heap、调用栈、metamethod 和 coroutine scheduler。标准库不反向依赖 VM，因此没有循环依赖，也没有把 runtime owner 暴露给任意回调。

## Catalog 与稳定 builtin 身份

每个 Rust builtin 仍是可复制、可哈希的 `NativeFunction`，但按 base/math/string/table 分类。函数值不捕获 Rust closure；身份稳定，能放入 Lua table key、比较并由 GC 当作 primitive 处理。

```rust
{{#include ../../code/v16-core-stdlib/crates/lua_core/src/lib.rs:stdlib_native_ids}}
```

`lua_stdlib` 的 `FunctionSpec` 和 `LibrarySpec` 是唯一公开目录。VM 遍历 catalog 创建 library table，安装 math 常量，并为 string primitive 建立 `__index = string` 的 type metatable。因此 `string.sub(value, ...)` 与 `value:sub(...)` 走同一函数对象。

Base 直接安装到 global environment；math、string、table、coroutine 作为表安装。`_VERSION = "Lua 5.1"` 也在这里建立。

## `_G` 必须是真正的全局表

v0.15 用 Rust `HashMap<ByteString, Value>` 保存 global。若只额外创建一个 `_G` table 快照，`answer = 42` 与 `_G.answer = 42` 会修改两份数据，这是静默的语义分叉。

本版让 Heap table 成为唯一 global environment：

- `GETGLOBAL` 等价于 raw table get；
- `SETGLOBAL` 等价于 raw table set，nil 仍删除 key；
- `_G` 指回这个 table 自身；
- GC roots 只需包含 global table handle，其内部图由 collector 正常追踪。

这次迁移同时验证了 self-cycle 不会妨碍完整或增量 GC。

## 参数适配器与错误协议

Lua C 库的 `luaL_checknumber` 接受 number，也接受可转换的 string；string 函数通常也接受 number 并先转成字节文本。若每个 builtin 自己 match，很容易出现不同函数对同一输入做不同决定。

```rust
{{#include ../../code/v16-core-stdlib/crates/lua_stdlib/src/lib.rs:host_args}}
```

`Args` 提供：

- `required`：缺参产生一基 `bad argument #N`；
- `number` / `integer`：执行数字字符串 coercion 与范围检查；
- `bytes`：返回拥有所有权的 bytes，绝不假定 UTF-8；
- `table`：校验 typed handle；
- `optional_integer` 与 `tail`：统一缺省值和可变参数切片。

`HostError` 在 VM 边界加上函数名并分类为 `VmErrorKind::Type`。Heap、metamethod 或 Lua callback 错误保持自己的分类；多返回仍用 `Vec<Value>` 走 v0.10 的开放结果协议。

```rust
{{#include ../../code/v16-core-stdlib/crates/lua_vm/src/lib.rs:stdlib_adapter}}
```

## 本版公开的核心库

### Base

实现 `assert`、`collectgarbage`、`error`、`getmetatable`、`ipairs`、`next`、`pairs`、`pcall`、`print`、`rawequal`、`rawget`、`rawset`、`select`、`setmetatable`、`tonumber`、`tostring`、`type`、`unpack` 和 `xpcall`。

`pcall/xpcall` 在 Rust builtin 边界消费普通 `VmError`，返回 `false, message`；成功时在原结果前插入 true。这个受保护调用属于不可 continuation 的 Lua 5.1 C 边界，因此内部尝试 yield 会变成可捕获的错误。

`collectgarbage` 支持 collect/count/step/stop/restart，并接受 setpause/setstepmul 作为尚未调参的兼容入口。旧 `gc_*` 名称暂时保留给前几章的诊断示例，不宣称是最终 API。

### Math

公开 Lua 5.1 math 表的 28 个函数：常用三角/双曲/对数、`atan2`、`fmod`、`frexp/ldexp`、`max/min`、`modf`、`pow`、角度转换以及 `random/randomseed`；同时提供 `pi` 和 `huge`。

教学 RNG 使用固定的 64-bit LCG，使 seed 后的测试可重复，但数列不宣称与宿主 C `rand()` 相同。测试验证无参 `[0,1)`、单上界和双边界三种合同。

### Byte string

公开 `byte`、`char`、`find`、`format`、`len`、`lower`、`rep`、`reverse`、`sub` 和 `upper`。索引采用 Lua 的一基与负索引规则；`char/byte` 覆盖 NUL 和 0xff，所有操作保持 bytes。

本版 `find` 是明确的 plain byte search，尚未实现 Lua pattern；`format` 只公开并验证 `%s/%d/%i/%f/%q/%%`。ASCII 大小写不会误把无效 UTF-8 交给 Rust `String`。

### Table

公开 `concat`、`insert`、`maxn`、`remove` 和 `sort`。数组移动全部通过 Heap mutation API，从而保留 incremental write barrier。`sort` 教学实现采用 insertion sort，便于逐次调用 Lua comparator；比较回调同样禁止 yield，并传播结构化错误。

Coroutine 六个入口改由同一个 catalog 注册，执行状态机保持 v0.15 合同。

## 差分与测试策略

`tests/stdlib.rs` 不只检查函数存在：它构造公开函数矩阵，逐个验证成功值、数字字符串 coercion、空/负边界和错误参数。string 测试包含 NUL 与 0xff；table 测试覆盖原地移动、自定义 comparator 和混合类型错误；base 测试证明 `_G` 与普通 global 双向同步。

`examples/stdlib_diff.lua` 只使用两边已对齐的确定性语义。测试在系统存在 `lua` 命令时，让官方 Lua 5.1 和本 VM 执行同一 fixture 并比较数值结果；没有 oracle 时跳过外部进程，但固定期望仍由本地测试守住。

## 从 v0.15 迁移了什么

- 新增：`lua_stdlib` crate、函数 catalog、`Args` 与 `HostError`；
- 重构：builtin ID 按 base/math/string/table 分类；
- 重构：Rust HashMap global 变成 Heap-owned `_G` table；
- 新增：19 base、28 math、10 string、5 table 函数和 math 常量；
- 新增：string primitive method、`pcall/xpcall` 和标准 `collectgarbage` 入口；
- 新增：公开函数矩阵与可用时的官方 Lua 5.1 差分；
- 保留：v0.15 coroutine scheduler、GC roots 和禁止 yield 边界；
- 没有提前加入：文件 IO、os、package、debug 或 native module loader。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v16-core-stdlib/Cargo.toml -- `
  book/code/v16-core-stdlib/examples/demo.lua
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --workspace
```

示例组合 table sort、math、byte string 与 coroutine：

```text
type=number value=1616
```

本快照共有 109 个测试：74 个既有端到端测试、7 个标准库端到端/差分测试、7 个编译器测试、15 个 core 测试、3 个 stdlib 合同测试和 3 个 VM 测试。所有 workspace target 还必须通过 `cargo clippy -- -D warnings`。

## 本版本暂不支持

- string pattern engine，因此没有 `match/gmatch/gsub`，`find` 暂为 plain search；
- `string.dump` 属于 v1.1 binary chunk；`string.format` 尚无 flags、width、precision 和全部 specifier；
- base 的 `dofile/load/loadfile/loadstring` 等待 v0.17 loader，`getfenv/setfenv` 等待兼容收口；
- math RNG 序列不与平台 C `rand()` 对齐；非 ASCII case mapping 不承诺 locale 行为；
- `collectgarbage("count")` 暂按对象数而非 allocator KB，setpause/setstepmul 尚不改变阈值；
- `table.sort` 是教学 insertion sort，不是生产排序性能；
- io、os、package、debug 和 Lua 文件模块尚未实现。

## 章末练习

1. 新增一个无 Heap 副作用的 math 函数，只修改 enum 与 catalog，观察编译器如何指出 dispatch 漏项。
2. 让 `print` 统一调用 Lua `tostring` 而非快速 stringify，并为失败的 `__tostring` 写合同测试。
3. 为 `string.format` 增加宽度和精度 parser，禁止把 byte string 整体转换成 UTF-8。
4. 把 table insertion sort 换成稳定 merge sort，设计 comparator 报错时不破坏 table 的事务策略。
5. 把 allocation accounting 改成估算 bytes，让 `collectgarbage("count")` 返回 KB。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 `lua_stdlib` catalog、host adapter 和 scoped runtime access 是本版 crate 边界的扩展目标；
- [Value 与字节字符串](v01-value.md) 可回看为什么 string library 从入口就接收 bytes；
- [Table](v08-table.md) 可对照数组移动与 write barrier 的所有权路径；
- [Metatable](v12-metatable.md) 可定位 string method 与 protected callback 边界；
- 下一 checkpoint 是 v0.17：增加 io、os、package、debug、Lua 文件 loader、`package.path/preload/loaded` 与循环 require 合同。
