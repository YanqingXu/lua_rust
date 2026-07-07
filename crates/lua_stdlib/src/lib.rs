//! lua_stdlib — Lua 5.1 standard libraries
//!
//! Implementations of the 10 standard Lua 5.1 library modules:
//! base, math, string, table, io, os, coroutine, debug, package, test.
//!
//! ## Module Guide
//! - `catalog`: 标准库注册表和批量打开入口。
//! - `base`, `math`, `string`, `table`: Lua 5.1 常用基础库。
//! - `io`, `os`: 文件、进程、时间和平台相关库。
//! - `coroutine`, `debug`, `package`: 协程、调试和模块加载支持。

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod base;
pub mod catalog;
pub mod coroutine;
pub mod debug;
pub mod dump;
pub mod io;
pub mod math;
pub mod os;
pub mod package;
pub mod string;
pub mod table;
// pub mod test;

pub use catalog::{LibEntry, LibOpenFn, get_catalog, open_all};
