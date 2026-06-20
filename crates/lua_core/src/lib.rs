//! lua_core — Lua 5.1 interpreter runtime core
//!
//! Provides the foundational types, value system, GC infrastructure,
//! string interning, and core object model for the Lua 5.1 register VM.
//!
//! ## Migration Status
//! - Phase 1 target crate
//! - C++ reference: `lua_cpp/src/core/`, `lua_cpp/src/gc/`, `lua_cpp/src/common/`
//!
//! ## Module Map (C++ → Rust)
//! | C++ | Rust module | Status |
//! |---|---|---|
//! | `src/common/types.hpp` | `types` | ✅ P1.1 |
//! | `src/core/value.hpp/.cpp` | `value` | ✅ P1.1 |
//! | `src/core/gc_object.hpp` | `gc::header`, `gc::gc_object` | ✅ P1.2 |
//! | `src/core/gc_string.hpp` | `gc_string` | ✅ P1.2 |
//! | `src/core/string_pool.hpp/.cpp` | `string_pool` | ✅ P1.2 |
//! | `src/gc/garbage_collector.hpp/.cpp` | `gc::collector` | ✅ P1.2 |
//! | `src/gc/gc_strategy.hpp/.cpp` | `gc::strategy` | ✅ P1.2 |
//! | `src/gc/gc_mark.cpp` | `gc::mark` | ✅ P1.3 |
//! | `src/gc/gc_sweep.cpp` | `gc::sweep` | ✅ P1.3 |
//! | `src/gc/gc_finalize.cpp` | `gc::finalize` | ✅ P1.3 (骨架) |
//! | `src/gc/gc_weak.cpp` | `gc::weak` | ✅ P1.3 |
//! | `src/core/table.hpp/.cpp` | `table` | ✅ P1.4 |
//! | `src/core/function.hpp/.cpp` | `function` (Proto) | ✅ P1.4 |
//! | `src/core/upvalue.hpp/.cpp` | `upvalue` | ✅ P1.4 |
//! | `src/core/thread.hpp/.cpp` | `thread` | ✅ P1.4 |
//! | `src/core/userdata.hpp/.cpp` | `userdata` | ✅ P1.4 |
//! | `src/core/metatable.hpp/.cpp` | `metatable` | ✅ P1.4 |

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

// Phase 1.1: Types + Value system ✅
pub mod types;
pub mod value;

// Phase 1.2: GC infrastructure + String pool ✅
pub mod gc;
pub mod gc_string;
pub mod string_pool;

// Phase 1.4: Core object model
pub mod function; // ✅ P1.4 — Function: C/Lua closures, upvalue capture, env table
pub mod metatable; // ✅ P1.4 — TMS enum, metamethod lookup with flags caching
pub mod proto; // ✅ P1.4 — Proto: function prototype, bytecode, constants, debug info
pub mod table; // ✅ P1.4 — Table with array/hash parts and metatable
pub mod thread; // ✅ P1.4 — Thread: coroutine object, status mgmt, caller chain
pub mod upvalue; // ✅ P1.4 — Upvalue: open/closed, linked list, GC integration
pub mod userdata; // ✅ P1.4 — Userdata: GC-managed byte buffer, metatable, optional destructor
