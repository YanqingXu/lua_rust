//! lua_stdlib — Lua 5.1 standard libraries
//!
//! Implementations of the 10 standard Lua 5.1 library modules:
//! base, math, string, table, io, os, coroutine, debug, package, test.
//!
//! ## Migration Status
//! - Phase 4 target crate
//! - C++ reference: `lua_cpp/src/lib/`
//!
//! ## Module Map (C++ → Rust)
//! | C++ | Rust module | Status |
//! |---|---|---|
//! | `src/lib/lib_catalog.hpp/.cpp` | `catalog` | ✅ P4 |
//! | `src/lib/baselib.cpp` | `base` | ✅ P4 (20 functions) |
//! | `src/lib/mathlib.cpp` | `math` | ✅ P4 (26 functions via macro) |
//! | `src/lib/stringlib.cpp` | `string` | ✅ P4 (10 functions) |
//! | `src/lib/tablelib.cpp` | `table` | ✅ P4 (5 functions) |
//! | `src/lib/iolib.cpp` | `io` | pending |
//! | `src/lib/oslib.cpp` | `os` | pending |
//! | `src/lib/coroutinelib.cpp` | `coroutine` | pending |
//! | `src/lib/debuglib.cpp` | `debug` | pending |
//! | `src/lib/packagelib.cpp` | `package` | pending |
//! | `src/lib/testlib.cpp` | `test` | pending |

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod base;
pub mod catalog;
pub mod math;
pub mod string;
pub mod table;
// pub mod io;
// pub mod os;
// pub mod coroutine;
// pub mod debug;
// pub mod package;
// pub mod test;

pub use catalog::{LibEntry, LibOpenFn, get_catalog, open_all};
