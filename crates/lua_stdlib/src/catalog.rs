//! 标准库目录与注册 (Library Catalog)
//!
//! 管理 Lua 5.1 全部标准库的注册和查询。
//!
//! C++ 参考: `lua_cpp/src/lib/lib_catalog.hpp/.cpp`

use lua_core::gc::collector::GarbageCollector;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::state::LuaState;

/// 库打开函数类型（C 函数签名：返回栈上返回值数量）
pub type LibOpenFn = fn(&mut LuaState) -> i32;

/// 库模块打开函数类型
pub type LibModuleOpenFn = fn(&mut LuaState, &mut GarbageCollector);

/// 标准库注册条目
pub struct LibEntry {
    /// 库标识符
    pub id: &'static str,
    /// 库名（全局变量名）
    pub name: &'static str,
    /// 打开函数
    pub open: LibModuleOpenFn,
}

/// 获取所有标准库目录
pub fn get_catalog() -> &'static [LibEntry] {
    &[
        LibEntry {
            id: "_G",
            name: "_G",
            open: crate::base::open_base,
        },
        LibEntry {
            id: "math",
            name: "math",
            open: crate::math::open_math,
        },
        LibEntry {
            id: "string",
            name: "string",
            open: crate::string::open_string,
        },
        LibEntry {
            id: "table",
            name: "table",
            open: crate::table::open_table,
        },
    ]
}

/// 按 ID 查找库
pub fn find_library(id: &str) -> Option<&'static LibEntry> {
    get_catalog().iter().find(|e| e.id == id)
}

/// 打开所有标准库
pub fn open_all(l: &mut LuaState, gc: &mut GarbageCollector) {
    for entry in get_catalog() {
        (entry.open)(l, gc);
    }
}

/// 将栈顶的 C 函数注册为全局变量
pub fn register_global(l: &mut LuaState, _name: &str, _func_ptr: *const ()) {
    // Push the function name string — but we need GC access to create strings.
    // For now, register as a placeholder table entry.
    // The actual function pointer will be wired when C function calling works.
    l.push_nil(); // placeholder: function value
}

/// 注册 C 函数到当前栈顶表
pub fn register_function(_l: &mut LuaState, _name: &str, _func: LibOpenFn) {
    // TODO: actual lua_setfield equivalent
}

/// 注册库表（推入以 name 命名的空表）
pub fn register_lib_table(_l: &mut LuaState, _name: &str) {
    // TODO: push new table
}

/// 在 LuaState 的全局表中注册一个 C 函数
///
/// 使用 `std::mem::transmute` 将 C 函数指针转换为 Lua value。
/// 这是 Lua C API 的标准做法。
pub fn register_c_function(
    l: &mut LuaState,
    _name: &str,
    _func: LibOpenFn,
) {
    // Get or create the global table entry for `name`
    // For now, push a nil placeholder since C function calling is not fully wired
    l.push_nil();
    if let Some(ref global_tbl) = l.global_table {
        let tbl_ptr = global_tbl.as_ptr() as *mut Table;
        // SAFETY: global_table is a GC root; we have exclusive access via &mut LuaState
        unsafe {
            (*tbl_ptr).set(&Value::Nil, &Value::Nil); // placeholder
        }
    }
    let _ = l.pop();
}
