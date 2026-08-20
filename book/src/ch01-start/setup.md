# 搭建与运行

<div class="lesson-meta"><span>预计 10 分钟</span><span>动手实验</span><span>Rust 1.96+</span></div>

这一节只做一件事：确认解释器、教程和你的本地环境能够形成闭环。

## 1. 检查工具链

在仓库根目录运行：

```powershell
rustc --version
cargo --version
```

仓库通过 `rust-toolchain.toml` 固定工具链。进入目录后，`rustup` 会自动选择对应版本。

## 2. 执行第一段 Lua

```powershell
cargo run -q -p lua_app -- -e "local answer = 40 + 2; print(answer)"
```

预期输出：

```text
42
```

这条命令已经走过 `Lexer → Parser → Codegen → VM → 标准库 print` 的完整路径。后续章节只是逐层打开这条流水线。

## 3. 看见生成的字节码

```powershell
cargo run -q -p lua_bytecode -- examples/arithmetic.lua
```

你会看到 `LOADK`、算术指令和 `RETURN` 等操作码。现在不需要理解每个字段，只需记住：VM 执行的不是源码文本，而是一组紧凑指令。

## 4. 本地预览本书

安装与工作流一致的 mdBook 版本：

```powershell
cargo install mdbook --version 0.5.4 --locked
mdbook serve book --open
```

修改 `book/src/` 中的 Markdown 后，浏览器会自动刷新。单次构建使用：

```powershell
mdbook build book
```

生成结果位于 `target/mdbook/`，不会进入 Git 历史。

## 常见问题

### PowerShell 找不到刚安装的 mdBook

确认 Cargo 的 bin 目录已经加入 `PATH`，然后重新打开终端：

```powershell
$env:USERPROFILE + "\.cargo\bin"
```

### 第一次编译为什么比较慢

Cargo 正在下载并编译依赖。后续只重建改变的 crate，通常会快很多。

### 示例失败是否说明 Rust 环境坏了

不一定。项目是兼容性原型，某些 Lua 5.1 边界仍在实现中。先用上面的 `42` smoke test 判断基础链路，再查看错误所在章节或兼容偏差日志。

<div class="exercise">
  <strong>热身练习</strong>
  <ol>
    <li>把表达式改成 <code>(6 * 7) .. "!"</code>，观察 Lua 的拼接结果。</li>
    <li>创建 <code>examples/learning.lua</code>，写入一张表和一次循环，再通过 <code>lua_app</code> 运行。</li>
    <li>用 <code>lua_bytecode</code> 比较常量表达式与变量表达式产生的指令。</li>
  </ol>
</div>

下一章，我们从所有阶段都会经过的 `Value` 开始。
