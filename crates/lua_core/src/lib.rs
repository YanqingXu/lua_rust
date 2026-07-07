//! lua_core — Lua 5.1 interpreter runtime core
//!
//! Provides the foundational types, value system, GC infrastructure,
//! string interning, and core object model for the Lua 5.1 register VM.
//!
//! ## Module Guide
//! - `types` / `value`: Lua 值标签和值表示。
//! - `gc`: GC header、引用包装、标记、清扫、弱表与 finalizer。
//! - `gc_string` / `string_pool`: 受 GC 管理的字符串与字符串驻留池。
//! - `table` / `metatable`: 表结构、数组/哈希混合存储和元方法查找。
//! - `proto` / `function` / `upvalue`: 函数原型、闭包和 upvalue。
//! - `userdata` / `thread`: 完整用户数据和 Lua coroutine 对象。

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
