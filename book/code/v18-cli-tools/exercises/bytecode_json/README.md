# Lua 字符串是 bytes，为什么 JSON 常量要写 hex？

先预测：Lua 的 `0xff` 是哪个 Unicode 字符？把 `0xff` 和 `0xfe` 都经
`String::from_utf8_lossy` 后，还能区分原输入吗？把引号、反斜杠、NUL 和换行原样放进
JSON 引号里，得到的还是合法 JSON 吗？本包只设 **一个 encode_bytes TODO**。

从仓库根运行，先构建整个 workspace 的 binary：

```powershell
$toolManifest = 'book/code/v18-cli-tools/Cargo.toml'
cargo build --manifest-path $toolManifest --workspace --bins
cargo run --quiet --manifest-path $toolManifest --example bytecode_json_problem_chain
cargo test --manifest-path $toolManifest --test bytecode_json_reference
cargo test --release --manifest-path $toolManifest --test bytecode_json_reference
```

默认 6 项参考、1 项 ignored learner。example 缺省 `all`，也接受
`bytes|proto|real|document`；`document` 只输出真实 `proto_json` 的 JSON，其他模式
显示固定对照。默认共 12 行，不读取 starter；完成 TODO 不会把 before 一并修掉。
`real` 使用同 profile 的 `lua_bytecode` sibling，不从 PATH 寻找另一个程序，也不会
自动执行 nested cargo。缺 binary 会给出完整 build 命令。Release 示例前同样先运行
`cargo build --release --manifest-path $toolManifest --workspace --bins`。

## 先区分两种失败

`faults::raw_text` 在 value 的引号内直接拼文本。即使输入本来是合法 UTF-8，内含
`"`、`\`、NUL 和换行也会破坏 JSON 字符串边界；不能把“Rust String 可打印”当成
“已正确 JSON escape”。invalid UTF-8 更不能不经转换就成为 Rust String。

`faults::lossy_text` 则用不同的输入展示信息丢失：

```text
0xff → {"type":"string","value":"�"}
0xfe → {"type":"string","value":"�"}
```

这两个特定输出是合法 JSON 文本，却相同。再给文本补上正确的引号 escape，也不能
找回已经丢失的原字节。这里 raw/lossy 两个固定函数使用同一段坏拼接，输入分别隔离
“注入控制字符”和“不同 bytes 被合并”两个问题；不是声称加一层 escape 就解决二者。

## 唯一 TODO：精确 string/hex 对象

只修改 `starter.rs::encode_bytes(bytes: &[u8]) -> String`。输入是短暂共享借用，
输出拥有自己的 ASCII JSON 文本，不保留输入 slice；每个字节都要贡献恰好两位小写
hex，空串也有自己的完整对象。

```diff
-format!("{{\"type\":\"string\",\"value\":\"{}\"}}", String::from_utf8_lossy(bytes))
+// 每个 byte 用 {byte:02x}，然后写入固定 string/hex schema
+format!("{{\"type\":\"string\",\"hex\":\"{hex}\"}}")
```

例如引号/反斜杠/斜线 `[0x22,0x5c,0x2f]` 得到
`{"type":"string","hex":"225c2f"}`，NUL/0xff 得到 hex `00ff`，无需把它们解释成
Unicode scalar。hex 是当前查看工具选用的数据表示，不是 Lua 官方 binary chunk。

```powershell
cargo test --manifest-path $toolManifest --test bytecode_json_exercises bytes_constants_preserve_exact_bytes_as_lower_hex -- --ignored --exact
cargo test --manifest-path $toolManifest --test bytecode_json_reference bytes_constants_preserve_exact_bytes_as_lower_hex -- --exact
```

每条必须恰好执行 1 项。初始 learner 失败；完成后相同共享合同通过。合同不是
contains("hex")：`decode_constant` 要求完整固定对象形状、偶数长度、仅小写 ASCII
hex，并逐对解码回原 bytes。矩阵覆盖空、ASCII、引号/两种 slash、控制字节、非法
UTF-8、中文的 UTF-8 原字节，以及全部 0..255；另断言 ff 与 fe 不能碰撞。

`decode_constant` 只识别这一种常量视图对象，不是一般 JSON parser，更不是 Proto
反序列化。完成本题不意味着可以把整个 JSON 读回可执行的 Proto。

## 回到真实 Proto：逐字段观察现有 renderer

`real::fixture` 使用真实 `Proto`、`ByteString` 和 `Instruction`，直接交给未修改的
`lua_bytecode::proto_json`。默认参考用完整 expected document 比对，而非搜索某个
片段，包含：

- 13 个根常量：nil、true、有限数值 1.25、NaN/inf/-inf 和七行 bytes 矩阵；
  nested proto 另有 bytes `00ff`，不能遗漏递归层。
- NaN 与正负无穷在当前 schema 中是 `type:number` 的**字符串** value
  `"NaN"/"inf"/"-inf"`，没有输出非法的裸 JSON NaN/Infinity。
- code 的 pc/line/raw/op/a/b/c/bx/sbx 均比对实际值。Move 的 A=7/B=9/C=11，
  raw=75678144、Bx=4619、sBx=-126452；全 1 的 raw=4294967295 显示 INVALID_63，
  A=255/B=C=511/Bx=262143/sBx=131072，缺失 line 显示 0。

该手工 Proto 专用于查看 renderer，包含非法 opcode 和不完整 line 表，没有交给 VM
执行。它不证明这些字节码可运行，也没有把 view schema 当成完整 Proto 持久化格式。

```powershell
cargo test --manifest-path $toolManifest --test bytecode_json_reference real_proto_json_matches_bytes_nested_nonfinite_and_raw_code_schema -- --exact
```

### metadata 的 escape 与 Lua bytes 常量不是同一规则

当前 `escape_json` 逐 byte 处理 metadata，把不可直接输出的字节写成 `\u00XX`。
引号、反斜杠和常见控制字符另走短 escape。对 UTF-8 文本 `é`，实际 name 是
`"\u00c3\u00a9"`，JSON decoder 读到的是 `Ã©`，不是原来的 Unicode 字符 `é`。
所以本包称它为**字节拼写的 escape**，不宣称通用 Unicode 文本保真，也没有修复它。
Lua bytes 常量的 hex 则能逐字节还原，包括合法 UTF-8 的每个原始字节。

```powershell
cargo test --manifest-path $toolManifest --test bytecode_json_reference real_metadata_escape_preserves_byte_spelling_not_unicode_scalar_text -- --exact
```

## 真实 binary 与独立 JSON 检查

真实 process probe 使用含 nested function 的 Lua source，常量精确为
`00 ff 22 5c 2f 0a`。先从 compiler 产物读 ByteString 核对原 bytes，再分别从 stdin
和实际临时 `.lua` 文件运行 `lua_bytecode --format json`。两次 code=0、stderr 空，
stdout 与同 source 的 public renderer 逐字节相同。

```powershell
cargo test --manifest-path $toolManifest --test bytecode_json_reference real_bytecode_binary_stdin_and_file_equal_public_renderer -- --exact
cargo run --quiet --manifest-path $toolManifest --example bytecode_json_problem_chain -- document
```

integration 先用 `env!("CARGO_BIN_EXE_lua_app")` 定位同目录的 lua_bytecode；根包不能
直接使用其他 workspace package 的 CARGO_BIN_EXE。共享 `bytecode_process.rs` 只定位，
调用仍复用旧 `process::run` 的并发 pipe drain、写完 stdin 的 EOF 和五秒 child
deadline，没有复制 runner，也没有改变旧 helper。

Rust 的精确对象/整 document 比对不冒充独立 JSON 语法实现。维护者另在本机忽略证据
目录，用 Python 标准 `json.loads` 严格解析 `document` 及真实 binary 输出：拒绝裸
非有限数字和重复 key，检查 schema、nested 与 bytes.fromhex 后的原 bytes；这是
独立语法/数据对照，不是本轮新增一个 JSON parser 的教学目标。全快照验证还会再次
保存该交叉证据。临时文件使用共享唯一目录 helper，显式清理合并错误，不改 cwd；
fallback Drop 不再 panic。child fixture 有界且不启动后代进程。

理解题：为什么正确 escape 也不能逆转 lossy 已造成的碰撞？
实现题：只补 encode_bytes，显式 learner 恰好执行 1 项并通过。
调试题：为何 metadata 中 `é` 的表现与 Lua bytes 常量不同？再解释为什么
`type:number,value:"inf"` 不应被读取方直接当成有限 JSON number。
