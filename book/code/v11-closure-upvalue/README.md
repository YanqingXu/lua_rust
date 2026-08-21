# v0.11 · Closure 与 Upvalue

本快照把 v0.10 的“函数只引用 Proto”升级为真正的 closure：函数对象拥有独立身份和 upvalue 列表，嵌套函数可以读取、修改并跨调用共享外层 local。

新增 `GETUPVAL`、`SETUPVAL` 与 `CLOSE`。open upvalue 指向活跃栈槽；离开块、break、return 或 tail call 时关闭为独立值。numeric/generic for 每轮关闭捕获变量，与 Lua 5.1.5 行为一致。

```powershell
cargo run --manifest-path book/code/v11-closure-upvalue/Cargo.toml -- `
  --dump-bytecode book/code/v11-closure-upvalue/examples/demo.lua
cargo test --manifest-path book/code/v11-closure-upvalue/Cargo.toml --workspace
```

本版本暂不支持 metatable、GC、弱表和 coroutine。closure 自引用与对象环仍可能形成 `Rc` 环；v0.13 会用 Runtime 管理的 `GcRef` 取代这一阶段的共享所有权模型。
