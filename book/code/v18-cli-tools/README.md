# v0.18 · CLI、REPL 与字节码工具

本快照完成 Lua 风格脚本、stdin、`-e/-l/-i/--`、arg table 与持久 REPL，并新增 `lua_bytecode` crate。工具可从同一 Proto 生成 text、稳定 JSON、CFG DOT，或执行后输出逐指令 trace JSON。

```powershell
cargo run --manifest-path book/code/v18-cli-tools/Cargo.toml -- `
  book/code/v18-cli-tools/examples/demo.lua
cargo run --manifest-path book/code/v18-cli-tools/crates/lua_bytecode/Cargo.toml -- `
  --format cfg book/code/v18-cli-tools/examples/demo.lua
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace
```

CLI 的 usage error 返回 2，compile/runtime/IO error 返回 1，Lua `print` 与脚本副作用保持 stdout 合同。REPL 支持 `=expression` 和未闭合 block 的续行。

fixture manifest 与 process runner 对 stdout/stderr/exit code/timeout 做黑盒验收。v1.0 前仍需补齐语言/标准库差异并建立完整 oracle gate。

## CLI / REPL 的三条问题链

- [错误退出](exercises/cli_exit/README.md)：一个 finish TODO；打印错误却 exit 0、所有错误给 1、打印 script return 三个反例；真实进程另验输出、退出码与失败中止顺序。
- [跨输入 owner](exercises/repl_owner/README.md)：一个 submit TODO；一次性 API 每次新建 Session 丢失状态，复用同一 owner；实际读取 GC 后的 table/closure/package/registry，普通 chunk local 不跨输入。
- [buffer 与恢复](exercises/repl_recovery/README.md)：一个 submit TODO；未完成保留源码，成功或普通错误清空，Session 始终复用；保留历史启发式漏判，真实 binary 另按当前结构化协议验收。

从仓库根目录执行。示例真实进程探针寻找同 profile 的 binary，先 build，不在示例内
嵌套 cargo；Release 示例也先 build --release --bins。

```powershell
$toolManifest = 'book/code/v18-cli-tools/Cargo.toml'
cargo build --manifest-path $toolManifest --bins
cargo run --quiet --manifest-path $toolManifest --example cli_exit_problem_chain
cargo run --quiet --manifest-path $toolManifest --example repl_owner_problem_chain
cargo run --quiet --manifest-path $toolManifest --example repl_recovery_problem_chain
cargo test --manifest-path $toolManifest --test cli_exit_reference
cargo test --manifest-path $toolManifest --test repl_owner_reference
cargo test --manifest-path $toolManifest --test repl_recovery_reference
cargo test --manifest-path $toolManifest --test cli_exit_exercises -- --ignored
cargo test --manifest-path $toolManifest --test repl_owner_exercises -- --ignored
cargo test --manifest-path $toolManifest --test repl_recovery_exercises -- --ignored
cargo fmt --manifest-path $toolManifest --all -- --check
cargo clippy --manifest-path $toolManifest --workspace --all-targets -- -D warnings
cargo test --manifest-path $toolManifest --workspace
cargo build --release --manifest-path $toolManifest --bins
cargo test --manifest-path $toolManifest --workspace --release
```

原快照 129 项 + CLI/REPL 参考20项（8/5/7），工具包再增加22项（6/7/9），
runner 三包另增加18项参考（5/6/7）；结构化续行增加compiler6、实际协议6、分类7、
EOF5，全快照 **213 通过、12 ignored**。十一包共十二个 learner ignored，
stdio 两项、其余十组各一项初始失败，只改各 starter hook 完成；参考与固定 CLI 通过不等于 learner 完成。
维护者验证失败→临时套解通过→原字节恢复并刷新 mtime 后再失败，交付保留 TODO。

模型是独立错误设计，真实 Session/进程另作证据。旧 recovery 模型保留 message
三段子串匹配，`return 40 +` 在该模型仍漏判；当前真实 Repl 已使用 CompileErrorKind，
pending EOF 报原诊断一次并exit1。普通行错误恢复后闲时 EOF 可 exit 0，`-e`/脚本错误则 exit 1；runtime
副作用不回滚。无 script、无 actions 且未指定 `-i` 时，非终端 stdin 才默认一次
作为脚本读取；显式脚本 `-` 也读 stdin，已有 `-e` 不会再自动读管道。
脚本 return value 不自动打印。此前版本 GC 功能缺口未由本轮教学包修复。

共享进程 helper 并发 drain stdout/stderr，写完关闭 stdin，五秒 deadline 后只
kill/wait 当前 child；旧 tests 的小 fixture runner 仍先 wait 后读输出，不证明大输出
安全。永久复现入口是各包合同、real 与 examples；本机 target 证据不随 clone 交付。

## 字节码 JSON / CFG / trace 的三条问题链

- [bytes JSON](exercises/bytecode_json/README.md)：一个 encode_bytes TODO；raw JSON 与 lossy 碰撞反例，严格 schema/小写hex 解码回原 bytes；真实 Proto/嵌套/指令字段与工具文档对照。
- [CFG leaders](exercises/bytecode_cfg/README.md)：一个 leaders TODO；补齐 target 和分支/终止后 pc+1，按半开块及末指令生成 jump/next；保留不可达块，只处理当前 Proto。
- [执行前 trace](exercises/bytecode_trace/README.md)：一个 dispatch TODO；先复制当前 Value 再执行，比较 after/StackSlot alias 故障；真实 VM 验 before/分支/开关/失败边界及 identity 非深快照/GC root。

工具 binaries 属于 workspace 的不同 package，需先显式构建整个 workspace。示例
使用同 profile sibling，缺失时给出 build 入口，不嵌套 cargo，不从 PATH 另找工具。

```powershell
cargo build --manifest-path $toolManifest --workspace --bins
cargo run --quiet --manifest-path $toolManifest --example bytecode_json_problem_chain
cargo run --quiet --manifest-path $toolManifest --example bytecode_cfg_problem_chain
cargo run --quiet --manifest-path $toolManifest --example bytecode_trace_problem_chain
cargo test --manifest-path $toolManifest --test bytecode_json_reference
cargo test --manifest-path $toolManifest --test bytecode_cfg_reference
cargo test --manifest-path $toolManifest --test bytecode_trace_reference
cargo test --manifest-path $toolManifest --test bytecode_json_exercises -- --ignored
cargo test --manifest-path $toolManifest --test bytecode_cfg_exercises -- --ignored
cargo test --manifest-path $toolManifest --test bytecode_trace_exercises -- --ignored
cargo build --release --manifest-path $toolManifest --workspace --bins
cargo test --manifest-path $toolManifest --workspace --release
```

三个新 learner 各初始失败一项；默认参考分别 JSON6、CFG7、trace9，通过参考不等于
完成 TODO。`examples/bytecode_views.lua` 可直接交给四个 --format：它返回42与含
NUL/0xff/引号/反斜线/换行的 bytes，不 print，便于独立解析 JSON/trace。

JSON 非有限 number 当前为字符串 NaN/inf/-inf，metadata 另按字节 escape；没有 Proto
decoder 或跨版本 schema 承诺。CFG 不是可达裁剪，不递归 nested Proto。trace step
来自 events 枚举，快照只 clone Value；typed identity 既非 Heap 深拷贝也不自动保活。
CLI trace 运行失败时不打印库中已录的前缀；Lua print 则会在成功 JSON 前混入 stdout。
模型、public renderer/VM、真实进程和 Python 标准 JSON parser 各自承担明确证据范围。

## Fixture runner 的四条问题链

- [stdin 与双流](exercises/runner_stdio/README.md)：两个独立 TODO，写完关闭 stdin
  才给 EOF；stdout/stderr 各自并行 drain，避免先读一条或先 wait 的等待环。
- [超时回收](exercises/runner_timeout/README.md)：一个 stop TODO，deadline 后
  kill 再 wait，kill 报错仍尝试 wait；实际 timeout 与清理 status 分开，保留 partial bytes。
- [证据归档](exercises/runner_evidence/README.md)：一个 archive TODO，原始 bytes
  与 expected 分开，独占 run/case 目录保留失败与超时；写档失败不能伪装为完整报告。

```powershell
cargo build --manifest-path $toolManifest --workspace --bins
cargo run --quiet --manifest-path $toolManifest --example runner_stdio_problem_chain
cargo run --quiet --manifest-path $toolManifest --example runner_timeout_problem_chain
cargo run --quiet --manifest-path $toolManifest --example runner_evidence_problem_chain
cargo test --manifest-path $toolManifest --test runner_stdio_reference
cargo test --manifest-path $toolManifest --test runner_timeout_reference
cargo test --manifest-path $toolManifest --test runner_evidence_reference
cargo test --manifest-path $toolManifest --test runner_stdio_exercises -- --ignored
cargo test --manifest-path $toolManifest --test runner_timeout_exercises -- --ignored
cargo test --manifest-path $toolManifest --test runner_evidence_exercises -- --ignored
cargo run --quiet --manifest-path $toolManifest --example runner_evidence_problem_chain -- export target/runner-evidence-review
```

默认运行固定 faults 与参考对照，故障子进程也由外层 owner 限时清理并 join readers。
export 显式保留新的 run 目录，重复运行不覆盖旧证据。Cargo 明确设置 default-run 为
lua_app；runner_fixture 是受控测试程序，只有有界输出且不创建后代。示例按同 profile
定位真实 binary，不嵌套 cargo；Release 也先 build --release --workspace --bins。

本章不声明任意大输出、孙进程/进程组回收、文件系统事务或断电恢复能力。
标准 Child 的 Drop 不自动 kill/wait；教学 owner 自己实现 backstop。try_wait(Some)
已回收正常终态，超时后则必须仍完成回收，再收集读取线程结果。两条流各保留顺序，
不推导跨流全局时间线。完整字段、修改范围、精确验收见三包 README。

## 结构化 REPL 续行与 EOF

- [分类](exercises/repl_classification/README.md)：一个 decide TODO，只借用真实
  compile 结果，按 CompileErrorKind 选择 Execute/Preserve/Reject；重放关键词、
  EOF位置和措辞变化反例。编译器的6项新增测试覆盖具体词法/语法分类分支。
- [EOF](exercises/repl_eof/README.md)：一个 finish TODO，消费原诊断一次并清buffer；
  实际 Repl 同Session、不执行未完成前缀、成功只执行一次，另有6项实际协议集成测试。

```powershell
cargo build --manifest-path $toolManifest --workspace --bins
cargo run --quiet --manifest-path $toolManifest --example repl_classification_problem_chain
cargo run --quiet --manifest-path $toolManifest --example repl_eof_problem_chain
cargo test --manifest-path $toolManifest --test repl_classification_reference
cargo test --manifest-path $toolManifest --test repl_eof_reference
cargo test --manifest-path $toolManifest --test repl_input_protocol
cargo test --manifest-path $toolManifest --test repl_classification_exercises -- --ignored
cargo test --manifest-path $toolManifest --test repl_eof_exercises -- --ignored
```

本轮改动真实 v0.18 compiler 与 src/ 的 Repl/CLI。compile 仍返回 Result<Proto,
CompileError>，错误新增 kind；Display 继续显示位置和message。NeedMoreInput 是当前
构造尚未完成的提示，普通脚本仍以Err返回；不证明每个任意前缀都有合法后缀，亦非
增量parser。普通 Invalid/runtime 错误清buffer后继续，不回滚已发生的副作用。

EOF空闲=0，pending只消费已保存的编译诊断并返回1，I/O Err独立返回1。最后没有LF
的输入片段不合成换行：未闭短串遇真实LF是Invalid，直接EOF仍未闭则pendingEOF=1。
只有空闲时移除单个shorthand `=`，`==42`不再被当成两次shorthand剥离。泛型writer
承接REPL自身输出，不捕获Lua print。旧 recovery 的模型/图标为历史迁移前设计，
其真实process对照按新协议更新，上一批证据仍归各自提交。
