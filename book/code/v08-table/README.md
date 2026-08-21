# v0.8 · Table

这个快照新增数组/哈希混合 Table、构造器、索引/成员访问、索引赋值和共享 `Rc<RefCell<_>>` 对象。

```powershell
cargo run --manifest-path book/code/v08-table/Cargo.toml -- `
  --dump-bytecode book/code/v08-table/examples/demo.lua
cargo test --manifest-path book/code/v08-table/Cargo.toml --workspace
```

本版本暂不支持：Metatable、迭代器和真实 GC。自引用 Table 会形成 `Rc` 环并无法回收，这是 v0.13 引入运行时 GC 的明确动机。
