# 只剩一个 pass bool，还能复查这次运行吗？

前置：能区分 stdout/stderr bytes、普通退出状态和监督器 timeout。约 20 分钟，完成
一个 archive TODO：把本次 Capture 的事实写成可独立检查的文件，再保存判定结果。
本包不改生产 CLI、旧 runner/helper，也不实现签名日志、数据库事务或崩溃恢复。

## 先预测，再看两个固定错误

子进程实际 stdout 是 `OUT\0\xff\n`，expected 却是其他六字节。若归档器把
expected 写进 stdout.bin，日后只读这个文件还能发现原始差异吗？再跑一次同名 case，
若固定写 latest/metadata.tsv，第一轮的 argv、退出码和 elapsed 去哪里了？

从仓库根运行，先构建当前 profile 的 runner_fixture：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example runner_evidence_problem_chain -- all
```

`faults|real|all`（默认 all）均使用唯一临时目录，正常路径显式 finish 清理。
固定 faults/solution 不调用 starter；CLI 退出 0 不表示 learner 已完成。
完整固定输出为：

```text
before expected-copy: saved_expected=true observed_preserved=false verdict=false
before latest: same_path=true first_metadata_lost=true
after observed: stdout_hex=4f555400ff0a stderr_hex=45525200fe0a exit=7 passed=false
after isolation: distinct_runs=true same_case=true collision_rejected=true
real archive: cases=3 verdicts=[false,true,false] normal_codes=[7,9] timeout_kind=timed-out timeout_reaped=true
```

第一种错误的 metadata 仍诚实记录原判定 false，但 stdout.bin 已被 expected 污染。
不能拿“文件存在/长度对/字段完整”代替真实内容验证。第二种错误的两个返回路径
相同，后一次 metadata 覆盖前一次；当前值再完整，也不能复查被覆盖的旧运行。

starter 只保存 summary.txt 的 pass bool，无法复查输入和两路输出，complete 会返回
missing captured payload 错误。两个可运行 faults 是 expected-copy 和 latest 覆写，
都属于人为局部设计，不是声称旧 runner 曾经这样归档。

## 一道独立 TODO：保存 observed，而不是 expected

接口是 `archive(root: &Path, evidence: &Evidence) -> Result<PathBuf,String>`。
框架已经准备真实 Capture、expected、run/case ID、binary、argv 和请求 stdin；只改
[starter.rs](starter.rs)，分别保存三份原 bytes，再交给固定框架完成元数据：

```diff
 let writer=CaseWriter::begin(root,evidence)?;
-writer.write_new("summary.txt",evidence.verdict().passed.to_string().as_bytes())?;
+writer.write_new("stdin.bin",&evidence.stdin)?;
+writer.write_new("stdout.bin",&evidence.capture.stdout)?;
+writer.write_new("stderr.bin",&evidence.capture.stderr)?;
 writer.complete(evidence)
```

[solution.rs](solution.rs) 使用相同接口。[contracts.rs](contracts.rs) 检查原 bytes、
version、metadata、不同 run 的同名 case、重复 run/case 拒绝覆盖，以及失败判定。
logical run-first/run-second 放在每次新建的临时 root 下；合同不依赖某个历史 target
目录、机器用户名或已导出的时间戳路径。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_evidence_exercises -- --ignored --exact archives_observed_bytes_and_same_run_metadata
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_evidence_reference
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_evidence_reference real_children_archive_raw_bytes_repeat_and_timeout_without_greenwashing -- --exact
```

初始 learner 运行恰好一项并失败在缺少 stdin.bin；完成后同一合同通过。默认 reference
共 **7 项**：共享 archive 合同、两种固定故障、I/O 失败与 incomplete、timeout 分类、
损坏 manifest/身份不一致拒绝、真实子进程归档。默认 ignored 或参考通过不能代替
显式 learner 验收。

## 文件与 metadata 格式

每个成功 case 的布局：

```text
root/
  run_id/
    case_id/
      stdin.bin
      stdout.bin
      stderr.bin
      binary-version.bin
      metadata.tsv
      manifest.tsv
```

run_id/case_id 只接受 1–64 个 ASCII 字母、数字、`_`、`-`。run 可含多个不同 case；
case 目录用 create_dir 独占创建，所有文件用 create_new；相同 run/case 再次归档返回
Err，不能截断旧文件。run 的 canonical 父路径必须仍是 archive root。

`.bin` 文件完全按 bytes 保存，不经过 UTF-8/lossy/log 转码。stdin.bin 保存**请求提交**
的输入，不证明 child 已消费全部 bytes。元数据要求 executable 是绝对、可表示为
UTF-8 的路径，argv 是 Rust String；不承诺原生非 UTF-8 argv/Windows 任意 OsString
无损保存。路径和 argv 的 UTF-8 bytes 用 lower hex 放进 TSV，避免 tab/newline 混列。

metadata.tsv 是每行 `key<TAB>value<LF>`，key 唯一、按 BTreeMap 排序。主要字段：

| 字段 | 含义 |
|---|---|
| schema / run_id / case_id | 本格式版本与本次关联身份 |
| executable.utf8.hex | 本次执行使用的绝对 binary 路径 |
| binary.version.file / binary.version.bytes | 原 version 输出所在文件及长度 |
| argv.count / argv.N.utf8.hex | 有序参数；N 为从 0 开始的索引 |
| stdin/stdout/stderr.file 与 .bytes | 对应原始文件名及长度 |
| elapsed.ns | Capture 的实测 Duration；包含启动/等待/清理，不是写死的 timeout 数值 |
| termination.kind | exited 或 timed-out |
| exit.code / exit.success / exit.status_debug.utf8.hex | 仅普通 Exited 的 OS 状态；无 code 用 none |
| cleanup.code / .success / .status_debug.utf8.hex | 仅 timeout 后 wait 收到的 OS 清理状态 |
| timeout.ready_seen / .alive_at_boundary / cleanup.reaped / .error.utf8.hex | 超时观察点与清理结果 |
| expected.exit / .stdout.hex / .stderr.hex | 明确分开的验收条件 |
| verdict.stdout_matches / .stderr_matches / .exit_matches / .passed | 由本次 Capture 与 expected 计算的判断 |

所有 bytes/文本 hex 都是表示方式，不是 hash。binary-version.bin 对真实 fixture 是
`runner-fixture v1\n`，表示其协议版本；绝对路径和 version 文本都**不能唯一证明
二进制内容身份**，本包没有计算或验证 executable hash。

manifest.tsv 同样使用 TSV，包含 schema、run_id、case_id、stdin/stdout/stderr/version/
metadata 的固定文件名引用，以及最后的 `state<TAB>complete<LF>`。读回时检查完整行、
重复 key、固定引用名、manifest/metadata/实际目录的 run/case 一致、payload/version
长度；完整验收再将读回 bytes 和 metadata 与原 Capture 逐项比对。

read_archive 的结构检查不是内容认证：相同长度的篡改可能通过结构检查。固定
expected-copy 就利用这一点，完整合同仍通过对照原 observed bytes 抓到污染；不能用
同一个错误归档值反过来生成其 expected，做成自证。

## 完成标记、I/O 失败与清理边界

CaseWriter 创建 case 后先写 INCOMPLETE；TODO 写三个 payload；complete 写 version
与 metadata，**最后创建 manifest.tsv**，随后移除 INCOMPLETE，全部成功才返回 Ok。
读取要求无 INCOMPLETE、完整 manifest 和关联文件一致。只看到一个 manifest 文件名
存在，不足以认定完整。

I/O 碰撞测试先写真实 payload，再让 stderr.bin 被目录占用；write_new 返回 Err，
部分目录和 INCOMPLETE 保留，manifest 不存在，reader 拒绝它。另将 root 指向普通
文件，archive 也返回 Err，不能改成 Ok/pass，更不能覆盖原文件。

这是正常 Result 路径的完成协议，**不是 atomic 事务或崩溃恢复**。分段写入可留下
部分目录，磁盘满也可能留下部分 manifest；只要 INCOMPLETE 尚在，reader 会拒绝。
写入没有 fsync/同步目录承诺，不能据此推断断电后的持久性或任意文件系统一致性。

CaseWriter 不在 Drop 中自动删除归档。测试和默认 CLI 才用唯一 TempDirectory，正常
路径显式 finish 合并主错误/清理错误，提前返回或 panic 有不 panic 的 Drop fallback；
递归清理前由共享 helper 检查 canonical 范围。export 路径由用户明确选择，成功与
失败留下的事实均不自动删除。

## 真实 capture：非零不是一律失败，清理成功也不是 timeout 通过

本包直接组合 runner_timeout 的真实 capture 和健康 stop，**不调用 learner**。
先运行 version，再跑 exit7、exit9 和 READY 后 100ms 的 stall，共四个子进程。
fixture 没有后代，stall 等待 stdin EOF，监督器持住 pipe 并在超时后 kill/wait。

exit 模式的 stdout 是六字节 `OUT\0\xff\n`，stderr 是六字节 `ERR\0\xfe\n`；
该模式不读 stdin，所以真实请求为空，避免 fast-exit/BrokenPipe 竞态。带 NUL/0xff
的非空 stdin 另在合成合同和真实 stall 请求中保存；不声称输出 echo 证明其消费。

exit7 的 expected 是 0，所以归档保留 code7、OS success=false、verdict=false；
同名 case 下一 run 的 exit9 声明 expected9，两路 bytes 也匹配，verdict=true，
同时仍如实保存 OS success=false。negative fixture 的预期非零可以通过，不应强行
把所有非零改判成基础设施错误。

stall 保存两路 partial 原 bytes、timed-out、READY/边界 alive 与 cleanup.reaped。
kill 后 wait 的状态只在 cleanup.*，绝不假装成程序自然 Exited(1)。模型另注入
cleanup code0 且两路完全匹配，判定仍为 false：完成清理不表示程序按时完成。

归档入口接收已经得到的 Capture；spawn/读取等基础设施 Err 若没有 Capture，本包
不会凭空制造输出事实。共享监督器对当前无后代/有界 fixture 提供清理，不承诺任意
进程树或任意 inherited pipe 的 join 上界。

## 导出可保留的本机证据

默认 all 的目录会清理；需要保留时显式选择 export root：

```powershell
$archiveRoot = Join-Path (Get-Location) 'target/my-runner-evidence'
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example runner_evidence_problem_chain -- export $archiveRoot
cargo build --release --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --quiet --release --manifest-path book/code/v18-cli-tools/Cargo.toml --example runner_evidence_problem_chain -- real
```

export 仅运行健康归档，使用新的 run IDs，打印三个实际 manifest 绝对路径；再次 export
产生新目录，不覆盖上一轮。一次 export 创建三个 run 目录：两个同名普通 case 各自
一个 run，timeout 再一个 run；不是一次 export 只有一个 run。集成测试用
`env!("CARGO_BIN_EXE_runner_fixture")`，example
只找同 profile sibling，不嵌套 cargo、不从 PATH 选另一个 binary。

target 是本机 ignored 证据，**不会随 clone 获得**。永久入口是仓库中的 fixture 源码、
合同、参考解和这些可复制命令，不把本次机器上的某个 run 目录写成完成条件。

理解题：为什么 timeout 后 wait 返回 code0 也不能判成正常通过？调试题：若保存
expected 代替 actual，同时再从它生成 report，哪些检查会变成自证？完成后应能独立
实现 archive，读回原 bytes，指出每个 run/case/binary/argv 的关联与完整性边界。
迁移时保留原始事实与判定分离；故障设计、合成 Capture 和观察代码留在教学包。
