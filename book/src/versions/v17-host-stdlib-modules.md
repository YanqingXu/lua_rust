# v0.17 · 宿主标准库与 Lua 模块

<div class="lesson-meta"><span>核心增量：可失败、需清理的宿主资源</span><span>5-crate workspace</span><span>checkpoint v0.17</span></div>

v0.16 的 math/string/table 函数主要处理内存中的 `Value`。文件、环境、时钟和模块加载不同：它们会接触平台状态，可能部分成功，且离开 Lua 作用域后仍需要关闭资源。本版增加一个核心能力：**让 Runtime 对宿主资源、加载缓存和调试视图拥有明确的生命周期与失败协议**。

## 文件不是普通 userdata 标签

文件值使用 `UserdataRef` 参加 Lua identity、metatable 和 GC；真正的 `std::fs::File` 保存在唯一 VM owner 的 `HashMap<UserdataRef, Option<File>>`：

```text
Lua userdata ──identity──> Runtime file entry ──owns──> OS handle
      │                           │
      └── per-object metatable    ├── Some(File): open
                                  └── None: closed tombstone
```

Heap userdata 新增独立 metatable edge，collector 会追踪它。共享 file metatable 的 `__index` 指回自身，安装 read/write/close/flush/seek；因此 `file:read()` 使用 v0.12 已实现的方法调用语义，不需要在 Parser 中添加特殊语法。

```rust
{{#include ../../code/v17-host-stdlib-modules/crates/lua_vm/src/lib.rs:host_file_resources}}
```

文件有三条关闭路径：

1. `file:close()` / `io.close(file)` 将 `Some(File)` 取走并保留 closed tombstone；
2. 完整或增量 Sweep 完成后，VM 移除已经 stale 的 userdata entry；
3. `Vm::drop` 先 clear 所有文件，再执行 Heap shutdown。

测试在 Windows 上实际删除对应文件，证明 handle 已释放，而不只检查 Rust flag。closed tombstone 让 `io.type(file)` 返回 `"closed file"`，并使再次 read/write/close 产生结构化错误。

## Byte-oriented io

`io.open` 支持 r/w/a 及其 `+` 和 `b` 组合，失败返回 `nil, message`。File methods 支持：

- `read("*a")`：读剩余全部 bytes，EOF 仍返回空 string；
- `read("*l")`：读行并移除 LF 和可选 CR；
- `read("*n")`：读取 ASCII number 并把 cursor 留在 token 后；
- `read(count)`：读取指定字节数，EOF 返回 nil；
- `write(...)`：string/number coercion 后原样写 bytes，并返回 file；
- `seek("set"|"cur"|"end", offset)`、`flush`、`close`。

测试文件包含 NUL 和 0xff，证明路径转成宿主字符串不代表文件内容也转 UTF-8。当前 Windows 教学边界要求 path 本身是 UTF-8；错误会显式返回，不做 lossy 路径匹配。

`io.read/write/flush` 连接进程 stdin/stdout。默认输入输出文件切换与 lines iterator 留在兼容收口，不用隐藏 iterator 状态假装已经完成。

## OS 边界

本版实现 clock、date、difftime、execute、getenv、remove、rename、time 和 tmpname。文件系统 API 遵循 Lua 风格：成功返回 true，平台失败返回 `nil, message`；不会把权限、文件不存在或锁冲突变成 panic。

为了不依赖宿主 locale，`os.date` 当前把 timestamp 映射为 UTC civil date，支持 `*t/!*t` 和 `%Y/%m/%d/%H/%M/%S/%c/%%`。从 Unix day 到年月日使用显式整数算法，epoch 测试固定为：

```text
1970-01-01 00:00:00, weekday=5
```

`os.execute(command)` 明确是宿主副作用边界：Windows 使用 `cmd /C`，Unix 使用 `sh -c`，返回进程状态。验收只调用无参数的 shell-presence 查询，不在测试中启动任意外部命令。

## Loader 复用同一编译器

VM 在 v0.17 依赖 `lua_compiler`，但编译器仍只依赖 core，不形成环。`loadstring`、`loadfile` 和 `dofile` 都调用同一 `compile → Proto → Heap Function` 路径：

- load 成功返回可多次调用的 Lua function；
- 语法/文件错误时 loadstring/loadfile 返回 `nil, message`；
- dofile 将加载错误作为 Lua error，并原样传播 chunk 的多返回。

这也确保动态 chunk 与主脚本共享 `_G`、registry、GC 和 coroutine scheduler，而不是启动第二个 VM。

## `require` 的状态机

package table 暴露 `path`、`preload` 和 `loaded`。require 的顺序是：

```text
package.loaded[name] truthy? ──是──> 直接返回缓存
            │否
            ▼
package.preload[name]? ──否──> path 模板替换 ? ──> 读文件并编译
            │                         │失败
            ▼                         └──> 汇总所有候选诊断
 loaded[name] = true  （循环 sentinel）
            │
            ▼
       loader(name)
        │成功                         │失败
        ▼                             ▼
非 nil 结果写缓存；nil 保留/读取缓存    删除 sentinel，允许修复后重试
```

```rust
{{#include ../../code/v17-host-stdlib-modules/crates/lua_vm/src/lib.rs:lua_module_loader}}
```

模块名中的 `.` 转成 `/`，每个以分号分隔的 path template 都替换 `?`。测试包含 preload、同一模块只执行一次、A/B 循环依赖、缺失模块诊断，以及“第一次 loader 报错、修改文件后同一 VM 重试成功”。

动态 native module 没有被悄悄接受：`package.loadlib` 属于 v1.3，v1.0 只要求 Lua 文件模块。

## Debug 视图

debug 库公开本阶段能由真实 Runtime 数据支撑的入口：getinfo、getmetatable、setmetatable、getregistry、getupvalue、setupvalue 和 traceback。

- getinfo 接受 Lua/native function 或活动 stack level，读取 Proto debug name、upvalue 数和 line；
- getupvalue/setupvalue 直接访问 Closure 的 typed upvalue handles，并返回编译器记录的名字；
- debug metatable API 绕过 `__metatable` 保护，覆盖 table、userdata 和 primitive type；
- registry 返回 v0.13 已作为 GC root 的真实 registry table；
- traceback 基于当前 call-info 生成 byte string。

当前还没有 lexical local name/location 表、hook event 或 function environment，所以相应 debug API 不注册，而不是返回猜测值。

## Rust 知识映射与设计选择

- `Option<File>` 同时表达 open resource 与可观察的 closed userdata；
- file metatable 是 Heap graph edge，宿主资源 map 是 Runtime ownership edge，两者职责不混淆；
- `OpenOptions` 把 Lua mode string 翻译为显式权限组合；
- `Result<Vec<Value>, VmError>` 表达 Lua error，`Vec<Value>{nil,message}` 表达普通平台失败；
- `package.loaded = true` 是事务进行中的 sentinel，错误时 rollback；
- loader callback 在禁止 yield 的宿主边界执行，避免暂停一半的 require transaction；
- VM Drop 先 drain 宿主资源，保证确定性 shutdown。

替代方案是把 `File` 直接塞进 Heap object，但会迫使 core 依赖 `std::fs`，破坏 runtime value 与宿主能力的边界。另一个方案是全局 integer fd；typed userdata identity 更自然地复用 metatable、GC 和错误分类。

## 从 v0.16 迁移了什么

- 新增：io/file、os、package、debug builtin catalog；
- 新增：Userdata 独立 metatable 与 GC tracing edge；
- 新增：open file owner map、三条资源 drain 路径和二进制 IO；
- 新增：UTC time、环境、文件系统、shell status 与平台错误协议；
- 新增：loadstring/loadfile/dofile 共用编译器；
- 新增：package.path/preload/loaded、Lua file require、循环 sentinel、缓存与 rollback；
- 新增：Proto/upvalue/registry/metatable/traceback debug 视图；
- 保留：v0.16 核心库矩阵与 v0.15 coroutine/GC 合同；
- 排除：动态 native module、C API 和 binary chunk。

## 运行与验收

```powershell
cargo run --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml -- `
  book/code/v17-host-stdlib-modules/examples/demo.lua
cargo test --manifest-path book/code/v17-host-stdlib-modules/Cargo.toml --workspace
```

示例 require 一个只加载一次的 Lua module，写入并重读临时文件，再用 debug.getinfo 检查 module function：

```text
type=number value=1717
```

本快照共有 119 个测试：74 个主线端到端、7 个核心标准库/差分、10 个宿主标准库与模块测试、7 个编译器、15 个 core、3 个 stdlib 合同和 3 个 VM 测试。新增测试使用进程唯一临时目录，并在成功路径后逐项清理。

## 本版本暂不支持

- io.input/output 默认流切换、io.lines/file:lines、io.tmpfile 和 setvbuf；stdin 的 `*n` 尚未实现；
- os.exit、setlocale，以及 os.time(table)；os.date 不是宿主 localtime/locale 的完整 formatter；
- package.loaders/cpath/loadlib、`module(...)`/package.seeall 和动态 native module；
- debug.getlocal/setlocal、gethook/sethook、debug.debug、getfenv/setfenv；
- loader 的 chunk name 仍是教学 Proto name，traceback 尚未携带规范化 `@path`；
- Windows host path 当前要求 UTF-8，未覆盖任意宽字符路径；
- v0.16 尚未实现的 Lua pattern 与完整 string.format 仍未补齐。

## 章末练习

1. 给 file userdata 增加 `__gc` callback，与 VM sweep 后 prune 比较调用顺序和重入风险。
2. 实现 file:lines iterator；明确 iterator 如何拥有 file handle、EOF 后何时自动关闭。
3. 在 require 的 loader 执行前后制造 GC checkpoint，证明 package tables、loader function 和循环 sentinel 都是 roots。
4. 给每个动态 chunk 写入 `@path` debug name，让 traceback 显示模块来源。
5. 把 UTF-8 host path adapter 换成 Windows `OsString`/Unix raw bytes 的平台 trait，并设计跨平台测试。

## 与成熟实现和专题章节的对应

- 当前成熟实现的 scoped runtime access、HostIo、package searcher 与 debug hooks 是本版显式 owner map 的扩展目标；
- [Runtime 所有权与 GC](v13-runtime-gc.md) 可回看为什么 registry/package/file metatable 必须进入 root inventory；
- [Coroutine](v15-coroutine.md) 可对照 require/file callback 为什么属于禁止 yield 的宿主边界；
- [核心标准库](v16-core-stdlib.md) 可回看 catalog 和 HostError 如何扩展到平台失败；
- 下一 checkpoint 是 v0.18：完成 CLI/REPL 合同，并新增 `lua_bytecode` crate 提供 text/JSON、CFG、trace、fixture manifest 与超时 runner。
