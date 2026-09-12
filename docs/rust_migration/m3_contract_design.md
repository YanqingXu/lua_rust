# M3 首批合同设计（2026-09-12）

状态：设计草案。此文不宣布 M1/M2 全部完成，不新增已发布 ABI，不将当前
`string.dump`/`package.loadlib` 的 unsupported 改成伪实现。前置验收和开放项以
[审计整改状态](../audits/2026-09-12-remediation.md)为准。

## 首个切片：格式和验证器先于对外入口

固定 C++ `87c15e69ceb94eb74e28226ccbefb7e196635711` 的 dump **并非 stock Lua
5.1 dump 的相同编码**。读取其 `src/lib/stringlib.cpp:1411–1560` 与
`src/lib/baselib.cpp:880` 的 writer/reader 后，两个编码必须分别建模：

| 字段 | 官方 Lua 5.1.5 | 固定 C++ project-local |
| --- | --- | --- |
| 前 12 字节 | Lua 5.1 版本、端序、int/size_t/Instruction/Number 尺寸 | 同类 header，随后另有 `LC++` 四字节标记 |
| 字符串 | size_t 长度含尾 NUL；0 表示缺省来源等 NULL 值 | u32 长度不含尾 NUL；0xffffffff 表示缺省 |
| 数字与计数 | 受 header 声明的平台布局约束 | writer 显式小端 u32/u64，数字保留 f64 位型 |
| Proto 四个 byte 字段 | nups、numparams、vararg、maxstack | numparams、vararg、maxstack、nups |
| LocVar | name、startpc、endpc | 额外写入 reg |

本机两个已固定 SHA 的 oracle 执行 `string.dump(function() return 1 end)`：
C++ 为 104 bytes、官方为 105 bytes，前 12 bytes 均为
`1b4c75615100010408040800`；C++ 随后的四字节为 `4c432b2b`。来源字符串为
`=(command line)`，不能将此长度当成所有平台的 golden。
本地最小证据：`target/m3-format-assessment/headers.json`，含源码、完整 stdout、
stderr、exit 和 executable SHA。此轮只观察 dump 字节，没有把其中一个格式盲目
交给另一种未经加固的 loader。

因此首批实现建议提供明确的 `ChunkFormat` 区分。loader 按经过验证的 header 与
project-local 标记选择 reader，不能把解析失败当作理由反复尝试别的格式。默认 dump
按项目目标拟选择固定 C++ 编码；stock 导出应是独立且可测试的能力。正式对外选择仍
遵循 plan 的双 oracle 决策规则，不能仅因两个 header 相同而宣称互通。

## 建议代码边界

1. `lua_compiler::binary` 先把受限字节输入解成**不含 GcRef 的拥有型数据**；来源、
   常量字符串、局部变量/upvalue 名称保持任意字节，数字保存原始位型。
2. 在发布到 Runtime 前，验证整棵树的预算、字段和指令合同。通过验证的数据才可以
   使用显式 collector/StringPool 与 publication root 构建 managed Proto。
3. 失败必须释放本次临时数据和已发布临时根，不改写正在运行的 caller，不留下部分
   function 或服务注册。禁止输入携带地址、ObjectId、StateHandle 或进程内 registry ID。
4. serializer 从 scoped collector access 复制/读取经过验证的 Proto 树，写入受限
   输出；writer 失败后停止后续调用。不可跨宿主 callback 保留受管 payload 引用。
5. 最后接入 `load`/`loadstring`/reader/CLI 与 `string.dump`，共用一种验证入口，
   不在某个入口添加绕过 verifier 的快速路径。

`LocVar.reg` 是两种格式的真实差异：stock 输入没有该字段。调试名称恢复必须使用
明确的 stock 局部变量约定，不能以 0 或 -1 填充后假称全部 debug parity 已通过。

## 验证器最低合同

| 层级 | 必须检查 | 失败证据 |
| --- | --- | --- |
| 输入/预算 | 总 bytes、checked 长度加乘、剩余输入、总 Proto/指令/常量/字符串/debug 数量、深度 | 截断、负计数、超预算、尾随数据、未知 header 的逐类受控错误 |
| Proto | maxstack/参数/vararg/upvalue、lineinfo 长度、局部作用域边界与名称 | 不可构造无效 call window；来源缺省继承单独验证 |
| 指令 | opcode、寄存器/RK/常量/子 Proto/upvalue 索引、jump 目标 | 按 opcode 语义校验，不能只检查整数是否位于某个全局上限 |
| 多字指令 | SETLIST 扩展 word、CLOSURE upvalue 绑定、跳转不能落入数据/绑定区域 | 合法多字序列接受；畸形序列在 VM 前拒绝 |
| 生存期 | GC/failure injection 中途退出、结果 publication、close | 默认 Miri/ASan 定向测试，错误后原 Runtime 仍能执行和归零 |

资源限制应成为显式参数并保存到 artifact，不能在 benchmark 或测试里隐藏增大。
在完整 allocator callback/可恢复 OOM 实现前，只能宣称有输入与请求容量预算，
不能宣称所有系统分配失败均可恢复。

首批 golden 至少覆盖 nested Proto、捕获 upvalue、无/有 debug、embedded NUL/invalid
UTF-8、±0/非有限数、vararg、SETLIST 扩展和空函数；每项记录 producer SHA、格式、
参数及原始 bytes。跨进程 roundtrip、允许的双向格式互通与原始字节相等分开统计。
官方 `calls.lua` 和完整 `all.lua` dump/undump 只有真正执行到完成点才可移出开放项。

## C API 的前置切片

已有 Rust native callback 使用 `*mut c_void -> i32` 和内部请求/错误约定，**不是已实现
的 Lua 5.1 C API**。后续 `lua_capi` 应使用 opaque `lua_State`，不暴露 Runtime、GcWrite、
Vec 或 Rust enum 布局。对象和 state 必须绑定 Runtime、线程和生命周期；字符串指针的
有效期、stack/pseudo-index、跨 state 移动和 close 后禁止访问需逐项定义。

最关键的设计阻塞是非局部错误和 yield：不能让 C longjmp 越过持有 Rust 借用或 Drop
guard 的帧，也不能让 Rust panic 越过 `extern C`。`catch_unwind` 不会捕获 longjmp，
不能把它作为唯一边界。首批需给出独立 C shim/受控调用边界的设计与最小 C consumer，
再移植 call/pcall/error/yield，而不是先导出同名符号。

固定 C++ 的 `tests/compatibility/lua51-public-api-contract.json` 声明 80 个 lua.h、
34 个 lauxlib.h、9 个 lualib.h 函数，共 123 项。迁移时 Rust 初始状态须逐项记为
UNSUPPORTED；不得复制 C++ 的 PASS。每次提升状态必须有纯 C 编译、独立链接/调用、
错误/allocator/生命周期差分证据。static/shared link 与 export 列表均需验证。

## Native module 与退出条件

动态库 handle 必须在全部 C closure、userdata destructor 和 finalizer 不再可能调用
该库之后释放。模块缓存、重复加载、symbol 失败和部分初始化回滚都要由 Runtime
统一拥有；卸载次序接入现有显式资源关闭链。完成公开 C ABI 与调用错误边界之前，
`package.loadlib` 保持明确 unsupported。

建议首个可合并交付仅包含：固定格式 golden、拥有型 reader、预算及 verifier、
失败原子性测试。其后依次交付 managed Proto 发布/serializer、Lua 入口、C ABI
最小 consumer、123 项合同、动态模块生命周期。这样每个切片都有独立验收，不将
尚未稳定的 Rust 内部边界固化为外部 ABI。
