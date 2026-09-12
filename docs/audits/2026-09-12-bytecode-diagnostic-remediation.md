# bytecode 工具诊断字节整改

本工作包补齐此前 [诊断字节整改](2026-09-12-diagnostic-bytes-remediation.md)
未覆盖的 `lua_bytecode` 出口。固定 C++ oracle 仍为
`87c15e69ceb94eb74e28226ccbefb7e196635711`，成功输出和参数合同保持原范围。

## 固定合同与实现

84 个隔离进程的红基线确认：C++ 的 Text、CFG、Diff 编译错误在 stderr 保留
`0xFF`，Rust Debug/Release 则取 Parser 的宿主 String 视图，改成 ASCII `\xff`。
对单独的原始 FF 字符串 token 和字面 ASCII `\xff` token，两边错误正文相同，
只有 token 字节不同；旧 Rust 把二者压成相同输出。

`ToolError(Vec<u8>)` 现在从 ParseError/CodegenError 的 `message_bytes()` 获取
完整消息，贯穿 `run_tool` 传到异常输出出口。宿主 IO、序列化错误仍按原文本转入。
Text、CFG、Diff 左右编译失败都保持 stdout 空、stderr 诊断、正常退出码 3。

NUL 另有已实测的边界：固定 C++ 将 `e.what()` 当 C 字符串输出，在首个 NUL
截断。第一版 FF 修复保留了 NUL 后的内容，与 C++ 不同。最终版本只在 bytecode
异常输出处取首个 NUL 之前的 payload，再添加既有换行；内部错误仍保留完整
字节，Parser、Runtime 和通用 writer 不截断。

JSON 是既有 Rust 扩展；C++ 在读源码前拒绝 `--format=json`，不能将该参数错误
视为 C++ 的 JSON 编译诊断。Rust JSON 编译失败沿用文本 stderr 和退出码 3，
成功 schema 2 没有修改。

## 定向证据

原始 argv、输入、输出 `.bin`、hex、hash、退出状态及源码身份均保存在
`target/bytecode-diagnostic-evidence/`：

- 红基线：`observations.json`，84 次进程，无超时或异常退出。
- 第一版 crate 全测：7 unit、2 新回归、2 既有 JSON，共 11/11。
- 最终专属回归：`diagnostic_bytes` 3/3，覆盖 Text/full、CFG/full、Diff 左右、
  Rust JSON、FF 与字面转义区分，以及 NUL 截断；完整捕获输出，30 秒超时为失败。
- 最终 Debug 模式复验：`after-final-debug/observations.json`，29 次进程；18 个
  共有用例全通道原字节一致。另 6 个首 token FF 用例仍有既有词法正文差异，
  5 个是 Rust JSON 扩展，分别记录。
- NUL 修复前的三份输出在 `nul-boundary/observations.json`；最终 Debug 与
  固定 C++ 完全相同的结果在 `nul-boundary/final.json`。

首次 NUL 回归错误地命名为 Windows 保留设备名 `nul.lua`，读取为空文件而失败。
改为 `raw-nul.lua` 后通过，未放宽断言；失败日志
`cargo-test-final-diagnostics.log` 与通过日志 `cargo-test-final-diagnostics-verified.log`
均保留。临时目录先解析绝对路径并核对父目录，再交给测试清理。

这些定向 Debug 证据之后，联合冻结的完整 M0/M1 基础门均通过，Debug/Release
全 targets 执行通过，包含上述 3 项新回归及既有 JSON 测试。最终两种构建的
bytecode 工具合同各 15/15（12 跨语言、3 Rust 扩展）通过，产物为
`target/repl-break-bytecode-evidence/bytecode-debug/report.json` 与
`bytecode-release/report.json`。联合源码和四个二进制身份见
[当前整改状态](2026-09-12-remediation.md)；本批现随联合批次提交。
首 token 的 `Unexpected character` / `unexpected byte 0xFF` 正文差异仍开放，
不能因字节保真就宣告所有工具错误消息一致。
