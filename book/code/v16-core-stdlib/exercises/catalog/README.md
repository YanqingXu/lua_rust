# v0.16：函数已经实现，为什么 Lua 里仍然找不到？

本包只研究“声明目录怎样变成可调用的 Lua 绑定”。`FunctionSpec/LibrarySpec` 来自真实
`lua_stdlib`，函数值使用真实 `NativeFunction` ID，安装操作使用真实 `Heap` 的公开 API。
这里没有另写 `sqrt` 等函数体，也没有修改 VM 初始化。只修改 [starter.rs](starter.rs)
的一个 `install` TODO；[solution.rs](solution.rs) 是可选答案。

## 先预测，再重放

从仓库根目录执行：

```powershell
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example catalog_problem_chain -- missing
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example catalog_problem_chain -- identity
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example catalog_problem_chain -- duplicates
cargo run --quiet --manifest-path book/code/v16-core-stdlib/Cargo.toml --example catalog_problem_chain -- real
```

无参数或 `all` 会运行全部。CLI 固定调用 [faults.rs](faults.rs) 与参考安装器，不读取
starter。退出 0 只表示重放完成；即使 TODO 已修复，固定 before 仍显示错误。

```text
before missing: math.sqrt=nil
after installed: math.sqrt(81)=[Number(9.0)]
after binding_count=68
before wrong-id: math.sqrt(81)=[Number(81.0)]
after right-id: math.sqrt(81)=[Number(9.0)]
teaching preflight same namespace: duplicate math.same
cross namespace: left.same(81)=[Number(81.0)] right.same(81)=[Number(9.0)]
```

真实 catalog 已声明 `math.sqrt`，固定故障却在安装循环里跳过它。这个故障是独立的
安装实验，不代表当前真实 VM 正在漏装 `sqrt`。另一个故障把名字 `sqrt` 配给了 `Abs`
ID：值仍然是 function，调用也成功，却得到 81。因此只检查 `type(...) == 'function'`
不够；必须核对身份与可观察结果。

## TODO：按目录安装，不再保留另一份安装名单

`install(base, libraries)` 接收只读声明切片，并返回拥有 Heap/global table 的
`Installed`。输入使用真实 spec 类型，函数值仍是可复制 ID，不捕获 Rust closure。
安装器只做以下步骤：

1. 调用已提供的教学 `preflight`，检查名称冲突。
2. 创建 `Installed::empty()`，把 base 每条 spec 绑定到 `installed.global()`。
3. 为每个 library 创建独立 table，把其 `functions` 全部按原 name/ID 绑定进去。
4. 返回安装结果，通过 `?` 把 Heap/preflight 错误交回调用者。

可以从 [faults.rs](faults.rs) 的 `omit_sqrt` 看出最小差异：

```diff
  for &spec in library.functions {
-     if library.name == b"math" && spec.name == b"sqrt" {
-         continue;
-     }
      installed.bind(table, spec)?;
  }
```

不能把所有函数都放到 global、把不同库合并成一张表，或用固定 68 项代替传入的目录。
共享合同还会传入 `left.same/right.same` 的新目录，验证安装器真正处理输入。

```powershell
# 初始执行 1 项、失败；填完 TODO 后同一项通过。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_exercises catalog_installs_the_independent_public_contract -- --ignored --exact

# 8 项默认参考/故障/真实 VM 对照。
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_reference
```

默认 workspace 会显示一项 learner ignored。忽略不是完成，也不是隐藏当前 VM 的
功能回归。不要删除 ignored、改合同、改固定故障或改独立期望来通过练习。

## 不用同一份目录给自己证明完整

[expected.rs](expected.rs) 独立列出公开的 68 个 `(namespace, name, NativeFunction)`：
19 base、28 math、10 string、5 table、6 coroutine。它没有从 `BASE_FUNCTIONS` 或
`LIBRARIES` 生成。共享合同逐项检查 Heap 安装快照，最后检查总数，额外绑定也不能混入。

`independent_contract_detects_catalog_omission_and_wrong_id_mutations` 另把声明输入的
math library 移除，再把其 `sqrt` 改为 `Abs`。独立合同仍要求原来的名字和 ID，不会
随着输入减少而自动缩小成功范围。固定漏安装故障则是在声明仍正确时遗漏 `sqrt`。
两者分别覆盖“目录丢了声明”和“安装器漏了绑定”。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_reference independent_contract_detects_catalog_omission_and_wrong_id_mutations -- --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_reference faulty_id_is_callable_but_produces_the_wrong_result -- --exact
```

## 同一个库重复，与不同库同名，是两种输入

`math.same` 出现两次会覆盖同一 table key；教学 preflight 将它作为错误报告。base
重名、重复 library 名，以及 base 与 library 占用同一个 global 名也被拒绝。
`left.same` 与 `right.same` 属于两个表，允许分别绑定 Abs/Sqrt，调用 81 分别得到 81/9。

**preflight 是本实验新增的检查，不是真实 VM 已有的初始化检查。** 当前 VM 直接遍历
catalog 并 `table_set`，同名键会覆盖。已有 stdlib 的名字测试也没有替代这里的分 namespace
规则。不能把实验拒绝重复名的输出写成真实 VM 已经拒绝任意用户 catalog。

```powershell
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_reference teaching_preflight_rejects_duplicates_within_one_namespace -- --exact
cargo test --manifest-path book/code/v16-core-stdlib/Cargo.toml --test catalog_reference different_libraries_can_publish_the_same_member_name -- --exact
```

## 稳定 ID 怎样连接到真实函数体？

[real.rs](real.rs) 先从实验安装后的 Heap 表读取 ID，再编译含一个数字占位符的固定 Lua
程序，只把该常量替换为 `Value::NativeFunction(id)`。之后交给真实 `Vm` 执行。每段探针
校验只有一个占位符；它不是文本替换，也没有复制 builtin 分派逻辑。

对应调用是：`tonumber('42') → 42`、`sqrt(81) → 9`、二进制字符串长度 → 3、
`concat({3,1,2}, ',') → '3,1,2'`、`create` 后实际 resume → `true,42`。字符串包含
NUL/0xff。错 ID 的探针真实调用了 Abs，因此得到 81，不能靠名字猜测结果。

另一个探针完全通过真实 VM 自身安装的公开库执行同样效果。前一个验证“实验安装的
身份交给 VM 能执行”，后一个验证“真实 VM 的注册和调用保持可用”，二者不能混称为
向真实 VM 注入了漏安装故障。

## crate 职责与实验边界

- core：拥有 `Value`、typed handle、`NativeFunction` 和 Heap 数据/API。
- stdlib：拥有公开 catalog 和参数/错误规则，不在这里执行 Heap mutation 或保存 VM。
- VM：遍历声明安装绑定，拥有函数体 dispatch、Heap owner 与调用能力。

`gc_collect/gc_step` 等阶段诊断 helper 仍由真实 VM 单独安装，不在公开 catalog 的 68
项之内。实验安装器不安装它们；真实 VM 对照实际调用 `gc_collect()` 并验证 `gc_step`
可见，输出 `real VM separate diagnostics=true`。不要把“公开 catalog 集中”写成
“所有 NativeFunction 或所有初始化内容都在 catalog”。

实验只安装函数绑定，不复制 `_G/_VERSION`、math 常量、string type metatable、默认
roots 或自动 GC 调度；这些仍由真实 VM 负责。本包没有在安装过程中触发 GC，也不
证明任意 allocation failure 时的全量回滚、native closure、并发或 callback 资源协议。

理解题：名字存在、ID 正确和函数真正可调用，为什么需要分别验收？
实现题：只修 `install`，通过同一个显式 learner 合同。
调试题：把 `sqrt` 指向 `Abs`，为何 `type` 与无错误调用仍不足以证明成功？

完成时应能解释三个 crate 的职责，找出漏声明/漏安装/错 ID 的区别，并区分同库重复
与跨库同名。本机日志独立保存到 `target/teaching-v16-2026-10-15/catalog/`，为忽略产物，
不随 clone 提供；永久复现入口是本 README、合同、固定故障与例子源文件。
