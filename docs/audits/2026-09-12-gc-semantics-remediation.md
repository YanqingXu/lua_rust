# GC 回调与栈窗口修复记录

本轮承接深度审计后的 Runtime/GC 安全整改，修复真实 Lua 程序在自动回收时的假 yield、合法调用被根扫描拒绝、弱表保留过期临时值，以及定向 Miri 新暴露的 `setmetatable` 借用重叠。未修改根扫描范围、越界拒绝条件或弱表死亡判定，也未移动 oracle pin。

## 根因与改动

1. 自动回收通过 `ThreadStatus::Yield` 保存被中断 Lua 帧的执行位置。新 native `__gc` 回调继承了这个恢复标志，普通文件 finalizer 返回时被误判为主动 yield。`start_finalizer_callback` 现在在进入新回调前设置 `Ok`；回调实际发起的 yield 仍作为错误返回。
2. `LuaState::set_top` 曾把逻辑栈顶下降同步到物理初始化窗口。官方 `closure.lua` 出现活动帧需要 48 槽而物理窗口只有 27 槽，正确的 sweep 安全检查因此拒绝回收。现在逻辑收缩保留初始化窗口；补齐 `-1` 保持不变、`-2` 弹出一个、极小负数饱和到当前帧 base、新暴露槽为 Nil 的 Rust API 合同。
3. 新 native 帧过去只在 trace 中把非参数槽记作 Nil，真实槽仍保留之前作用域的临时对象。官方弱 `kv` 用例因此多留下一个循环中的表。现在普通 C 和 Runtime-native 帧都在 hook/continuation snapshot 前真实清空 `argument_top..ci_top`，与固定 C++ 的 native 帧初始化规则一致。保留完整根扫描，无需依赖局部变量调试信息推断存活范围。
4. `setmetatable` 曾在获得显式 `&mut GarbageCollector` 后，通过 `weak_mode` 再从 LuaState 动态取得共享 collector 引用，随后继续使用旧 mutable 引用。默认 Miri 报告 Stacked Borrows 违规。现在先完成模式查询，再开始显式 mutable collector 借用。
5. 公开 `call_value_with_results` 的临时调用区原本始于逻辑 top；宿主把 top 降到活动帧窗口内时，高位活值可能被调用参数覆盖。现在起点取 `max(saved_top, current_ci.top)`，结果发布仍恢复原逻辑 top。宿主回归覆盖 C `type` 和 Runtime-native `tostring` 两条路径。

主要实现位于 `crates/lua_vm/src/execute.rs`、`crates/lua_vm/src/state/lua_state.rs` 和 `crates/lua_stdlib/src/base.rs`。`runtime/{full_collection,root_trace,incremental_collection}.rs` 的最终逻辑没有改动。

## 验证

- 新增 `crates/lua_stdlib/tests/gc_semantics.rs` 的 7 项测试，以及 `crates/lua_app/tests/gc_semantics.rs` 的 80 次临时文件写入、定位、读取与关闭过程测试。Debug/Release 定向全部通过。
- 5 个 Lua 场景分别对固定 C++、官方 Lua 5.1.5、Rust Debug、Rust Release 验证，共 20 次运行均 exit 0、stderr 为空，同场景 stdout 一致。结果和二进制摘要：`target/audit-fixes-2026-09-12/gc-semantics-oracles/results.json`。
- 原有 1000 层 resume/wrap 链的 Lua 行为与 activation 深度断言通过。其旧测试曾明确依赖陈旧 wrapper 临时值保留一个协程，现在加强为回收后恰好零个协程；4 项 Runtime 关闭集成回归通过。
- 最终 stdlib 集成运行 130/131 通过。唯一失败为并行进行的错误来源改动给 `string.dump` 错误增加源位置，而旧断言仍期待无前缀；该项由错误来源整改处理，不属于 GC 失败。
- 默认 Miri 的初始失败与修复后运行分别保存在 `gc-semantics-miri.log`、`gc-semantics-miri-fixed.log`。后者验证 native/weak/根窗口修复的 5 项快照，5/5 通过，用时 711.68 秒。最终 helper/负索引 API 快照另以 `gc-semantics-host-miri-final.log` 验证，3/3 通过，用时 122.32 秒。两轮重叠一个根窗口用例，覆盖全部 7 项不同测试；不同快照不能合并表述为一次最终全量 Miri 运行。未关闭 Stacked Borrows 或其他 Miri 检查。

官方 `closure.lua` 和 `events.lua` 原脚本已完整通过。`gc.lua` 已通过弱表区段，Debug 在 200,000 层深结构测试处达到本轮 40 秒观察上限，Release 同处达到 90 秒上限且 stderr 为空；不能据此宣称完整官方 GC 脚本通过。Release 的独立观察结果记录于 `gc-semantics-official-release.json`，属于后续性能排查证据。

## 后续边界

当前 Rust 栈 API 对超出当前帧的负索引采用明确的饱和规则。大正索引的资源配额仍属于后续资源治理工作，本轮没有新增广泛 API 或静默调整配额。官方全套、完整 M0 与最终工作区质量门仍由主线统一复验。
