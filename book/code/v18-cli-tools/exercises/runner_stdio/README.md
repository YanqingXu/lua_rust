# stdin 的 EOF 与两个输出管道

本包启动真正的 `runner_fixture` 子进程，只让两个 hook 决定 stdin 的关闭时间与输出读取顺序。它不是 Lua VM，也不调用生产 CLI 的内部实现；旧 `tests/cli_contract.rs` 的小输出 runner 不能充当这里的大输出证据。

先在仓库根目录执行以下 PowerShell 命令。示例从自身所在 profile 目录定位 sibling `runner_fixture`，缺少二进制会明确报错，不会在运行中嵌套执行 Cargo。

```powershell
$cliManifest = "book/code/v18-cli-tools/Cargo.toml"
cargo build --manifest-path $cliManifest --workspace --bins
cargo run --manifest-path $cliManifest --example runner_stdio_problem_chain -- all
```

模式为 `stdin`、`dual`、`all`，默认 `all`。固定反例与参考方案直接使用 `faults.rs` / `solution.rs`，不受 `starter.rs` 的 TODO 影响。`all` 有五条报告：保留 stdin、关闭 stdin、先等退出、顺序读流、并行读流；三个错误方案均有看门狗终止报告，两个参考报告均为 `exact=true`、`timeout=false`。

## 问题一：write_all / flush 已返回，为何孩子仍不退出？

`stdin` fixture 先输出 `READY stdin`，做第一次 stdin read 后输出 `ACK stdin`，随后才 `read_to_end`。对于非空输入，ACK 只证明首个 byte 已被读取；它不证明整个 payload 已消费。空输入会在 EOF 后完成第一次零字节 read，同样输出 ACK。

父进程保留 `ChildStdin` 时，孩子读不到 EOF。反例在观察到 READY / ACK 后仍等不到退出，约两秒的宽松 watchdog 会 kill / wait 自己的孩子，再 join 所有 reader。参考方案在 write / flush 之后 drop 最后一个父端 writer，孩子才完成 read_to_end，回显包含 NUL / `0xff` 的原始输入，并在 stderr 输出 `EOF stdin`。验收比较两个流的完整 bytes，也覆盖空输入。

```diff
 let stdin = model::write_flush(stdin, input)?;
-Ok(Some(stdin)) // supervisor 继续持有 writer
+drop(stdin);    // 数据之后的 EOF 边界
+Ok(None)
```

## 问题二：为何先 wait，或读完 stdout 再读 stderr，都会卡住？

`dual` fixture 输出 READY，读一个启动 token 后输出 ACK，接着**先写 1 MiB stderr，再写 1 MiB stdout**。两个 payload 都有固定字节公式，合计有界，不会无限刷输出。

错误一只读取 READY / ACK 握手，随后等父进程 wait 完成才开始 bulk drain：父进程等孩子退出，孩子在写满的 stderr 管道上等读取。错误二读 stdout 到 EOF 后才读 stderr：孩子还没写完 stderr，因此无法关闭 stdout。参考方案在等待孩子退出之前启动两个 reader，分别排空两个管道，并 join 两者后才传播错误。

```diff
-let stdout = read_pipe(stdout, Some(notices)); // 阻塞到 stdout EOF
-let stderr = read_pipe(stderr, None);
+let output = thread::spawn(move || read_pipe(stdout, Some(notices)));
+let error = thread::spawn(move || read_pipe(stderr, None));
+join_pair(output, error) // 两个 reader 都 join 后再传播 Result
```

健康输出精确为 stdout `1_048_596` bytes（含 20-byte 握手）和 stderr `1_048_576` bytes。错误版在 kill 后只得到 stdout 握手与不完整的 stderr 前缀；Windows 同步写入在被 kill 后可能留下 **0 bytes**，不要求非空前缀，也不把特定部分长度当合同。握手、子进程存活与最终超时共同提供可观察证据，验收不比较精确耗时。若某平台管道容量足以容纳全部有界输出，错误版可能不阻塞，此反例会明确失败，不能据此外推所有 OS 的管道容量。

## 两个独立 TODO 与验收

只编辑 `starter.rs` 的 `send_input` / `start_drains`。第一题的合同固定使用参考 reader，第二题固定使用参考 sender，因此可以分别完成。默认测试不会运行 TODO；下面两条负向命令在初始状态各运行 **1 个** ignored 验收并失败，完成对应题目后应通过。

```powershell
cargo test --manifest-path $cliManifest --test runner_stdio_exercises stdin_send_then_close_delivers_eof -- --ignored --exact
cargo test --manifest-path $cliManifest --test runner_stdio_exercises parallel_readers_capture_both_streams -- --ignored --exact
```

以下两条正向命令分别调用同一份合同和参考 hook，每条必须运行 **1 个** 测试。

```powershell
cargo test --manifest-path $cliManifest --test runner_stdio_reference stdin_send_then_close_delivers_eof -- --exact
cargo test --manifest-path $cliManifest --test runner_stdio_reference parallel_readers_capture_both_streams -- --exact
```

整包有 **5 个默认 reference / 2 个 ignored learner**。另外三个默认 reference 验证固定错误版确实在握手后阻塞，并已 kill / wait / join。Debug 与 Release 运行方式如下；Release 示例运行前同样先构建该 profile 的所有 bins。

```powershell
cargo test --manifest-path $cliManifest --test runner_stdio_reference --test runner_stdio_exercises
cargo build --manifest-path $cliManifest --workspace --bins --release
cargo test --manifest-path $cliManifest --release --test runner_stdio_reference --test runner_stdio_exercises
cargo run --manifest-path $cliManifest --release --example runner_stdio_problem_chain -- all
```

## 进程与线程的所有权边界

`model::run` 持有 exact `Child`、可选 stdin writer 与 reader supervisor。正常路径 wait / try_wait 回收孩子后关闭 writer 并 join；Result 提前返回由 guard 先 kill / wait，再关闭 writer 并 join。`join_pair` 先 join 两个 reader，再用 `?` 传播各自错误，避免第一个错误让第二个 reader 脱离管理。反例等待门也会在孩子退出或被终止之后释放。

这里只覆盖本 fixture（没有后代进程）、有界输入输出和提供的 hook。它不声称清理任意 descendant、继承管道的其他进程、任意用户 hook 的永久阻塞或进程 abort。子进程的协议版本 `runner-fixture v1` 也不是可执行文件的加密身份。共享 fixture / locator / protocol 不属于这两个 TODO。
