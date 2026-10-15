# 到了 deadline，child 就已经停止并回收了吗？

本包只有一道 TODO：`starter.rs::stop(&mut dyn Lifecycle) -> Result<(), String>`。
先预测三个边界：只返回 timeout、只调用 kill、kill 后再 wait，在 hook 返回的瞬间
是否仍拥有 child？kill 与进程自行退出发生竞争时，kill 返回 Err 是否就该跳过 wait？

从仓库根运行固定对照，先构建 runner_fixture：

```powershell
cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --workspace --bins
cargo run --quiet --manifest-path book/code/v18-cli-tools/Cargo.toml --example runner_timeout_problem_chain -- all
```

模式为 `lifecycle|real|all`，缺省 all。示例始终使用固定 faults/solution，**不读取
starter**。正常参考包含 6 项，learner 为 1 项 ignored；默认通过或固定 CLI 退出0
不能代替显式 TODO 验收。

## 一个借用，只对这个 owner 的 child 操作

`Lifecycle` 只有 `kill` 和 `wait`，输入为 `&mut dyn Lifecycle`：hook 暂时独占借用
一个生命周期对象，通过 trait 同时接受可注入模型和真实 Child adapter。它不按进程
名枚举其他进程，也不持有 pid 列表。真实 adapter 始终操作同一个 `Option<Child>`。

固定模型可以在没有任何真实进程的情况下验证顺序；其 alive/reaped 布尔只是模型状态，
不能当作 OS 存活证据。真实 child 的 READY、try_wait、wait 和资源 owner 另外验证。

| hook 返回边界 | 步骤 | owner 仍存在 | wait 成功 |
| --- | --- | --- | --- |
| `return_without_stopping` | 无 | 是，模型仍 alive | 否 |
| `kill_without_waiting` | kill | 是，模型已经停止 | 否 |
| 参考 stop | kill → wait | 否 | 是 |

**观测点是 hook 返回时。** 两个故障随后由固定 backstop 对同一个 owner 做 kill/wait，
最终全部清理；它们证明边界尚未满足，不制造永久遗留进程。若只看最后 guard 清理后
的状态，三个设计会看起来一样，反例就失去意义。

`std::process::Child` 没有替你 kill/wait 的 Drop 实现；Rust 变量离开作用域不能作为
子进程已停止的证明。本包是自定义 Guard 显式执行后备清理。平台的进程资源、僵尸
进程表现不同，不把 Windows 的文件删除或路径变化当成 process 退出证据。

## TODO：kill 的结果与 wait 的责任分开

```diff
-Ok(()) // 只离开 timeout 分支
+let killed = child.kill();
+child.wait().map_err(|error| format!("wait after kill ({killed:?}): {error}"))?;
+Ok(())
```

先保存 kill 的 Result，再无条件尝试 wait。若最后一次 poll 之后 child 已自行退出，
kill 可能失败，wait 仍能取得状态并释放 owner；不能写 `child.kill()?; child.wait()?`
让第一个 Err 截断回收。wait 失败则返回带 kill 上下文的错误，owner 保留给 backstop
重试。这里只演示普通 Result 路径，不把模型注入错误当成实际复现了每种 OS 故障。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_timeout_exercises timeout_cleanup_always_attempts_wait_and_releases_exact_owner -- --ignored --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_timeout_reference timeout_cleanup_always_attempts_wait_and_releases_exact_owner -- --exact
```

两条命令各运行一项；初始 learner 应失败，完成后通过相同合同。合同覆盖正常 kill、
模拟 kill race（Err 后仍 wait 成功）和一次 wait 注入失败。最后一种情况必须保留
原错误，且在 backstop 之前仍有 owner，不能把“准备稍后回收”算作当前 hook 已完成。

已被 `try_wait()` 返回 Some 的 child 则属于另一条路径：该 Some 已收集退出状态，
框架释放 owner，不调用 timeout hook，也没有再次 wait 的要求。模型记录只含
`try_wait Some`；`wait_succeeded=false` 表示没有另行调用 Child::wait，模型的 reaped
已经为 true。真实普通退出0/7分别返回 Exited(ExitStatus)，都不经过 timeout 清理。

## 真实 child：READY 后确实还活着

`runner_support/fixture.rs` 的 stall 输出精确字节，然后阻塞在 stdin read_to_end：

```text
stdout: READY stall\nPARTIAL\0\xff\n
stderr: PARTIAL stderr\0\xfe\n
```

真实监督器并发 drain 两个 pipe，并保持 stdin 打开，让 EOF 不会提前结束 stall。
它在 stdout reader 读到 READY prefix 后才开始 100ms 主 deadline；READY 等待另有
五秒启动上限。主循环在每次 `try_wait()==None` 后检查 deadline，所以 TimedOut
对应真实仍未退出的 child，不是凭一段 sleep 猜“可能还在运行”。

超时时调用指定 hook，马上保存 boundary，再由 backstop 修复仍存在的 owner，最后
join 两个 reader 和 writer。return-only 的真实 boundary 额外 probe 仍是 None；
kill-only 不调用额外 try_wait，以免 Some 悄悄完成 reap，掩盖“没有显式 wait”的
观察边界。健康路径 boundary 为 kill/wait、owner=false、wait成功，backstop 无操作。

writer 持有 stdin 的安全 lease 为启动上限五秒 + 主 timeout + 五秒清理余量，覆盖
READY 之前的启动时间，避免 EOF 先于主 deadline 释放 fixture。显式监督清理完成后
发送 release，异常路径或 lease 到期也关闭 pipe。对本轮等待 EOF 的已知 stall
fixture，这给出额外退出后备。
Guard 的 Drop 还会对遗留 owner 尝试 kill/wait 并 join。fixture 无后代、输入和输出
有界；这些机制不承诺任意进程树、不可终止的系统调用或后代持 pipe 的通用 join 上界。

```powershell
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_timeout_reference real_ready_child_is_alive_then_killed_waited_and_preserves_partial_bytes -- --exact
cargo test --manifest-path book/code/v18-cli-tools/Cargo.toml --test runner_timeout_reference real_fixed_faults_are_observed_before_backstop_reaps_exact_child -- --exact
```

## Timeout 不是 Lua 自己退出1

新局部 `real::Capture` 保留 stdout/stderr 原 bytes，分类为
`Termination::Exited(ExitStatus)` 或 `Termination::TimedOut`。kill 后 wait 得到的
OS 状态另放 `cleanup_status`，绝不把它重新归类为脚本自己的 Exited(1)。两个故障也
返回 TimedOut；随后安全清理成功不表示被测程序已经按时完成。

`cleanup_error` 保存 hook 的普通错误，`hook_boundary` 与 `backstop_steps` 分别记录
前后边界；`final_owned=false` 证明监督器已释放这个 owner。所有真实 stall 对照逐
字节核对部分输出，NUL/无效 UTF-8 没被日志转码吞掉。这里只把原字节事实交给调用者，
归档和 expected-match 判定由另一个练习包负责；timeout 不能作为某个普通退出码匹配。

这是独立教学 capture API；旧 `support/process.rs` 保持原来的 Result<Output,String>
接口与行为，没有被本包替换。旧 helper 已先保存 kill 结果再 wait，这里补的是为什么
需要这个顺序及如何在反例边界验收。integration 使用 Cargo 提供的
`env!("CARGO_BIN_EXE_runner_fixture")`；示例按 current_exe 定位同 profile sibling，
需要先 build 对应 profile binaries，不嵌套 cargo，不从 PATH 另找程序。

理解题：为什么 kill Err 之后仍然需要尝试 wait，而 try_wait Some 之后不用再 wait？
实现题：只补 stop hook，显式 learner 恰好运行一项并通过。调试题：如果测试只看最终
guard 清理后的 owner=false，两个故障为什么也会“通过”？完成后应能沿 deadline →
hook 返回 → backstop → join 四个时点解释过程，并区分真实 Exited、TimedOut 和清理状态。
