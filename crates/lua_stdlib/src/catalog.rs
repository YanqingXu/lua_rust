//! 标准库目录与注册 (Library Catalog)
//!
//! 管理 Lua 5.1 全部标准库的注册和查询。
//!

use lua_core::function::CFunction;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::table::Table;
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
            id: "io",
            name: "io",
            open: crate::io::open_io,
        },
        LibEntry {
            id: "os",
            name: "os",
            open: crate::os::open_os,
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
        LibEntry {
            id: "coroutine",
            name: "coroutine",
            open: crate::coroutine::open_coroutine,
        },
        LibEntry {
            id: "debug",
            name: "debug",
            open: crate::debug::open_debug,
        },
        LibEntry {
            id: "package",
            name: "package",
            open: crate::package::open_package,
        },
    ]
}

/// 打开所有标准库
pub fn open_all(l: &mut LuaState, gc: &mut GarbageCollector) {
    for entry in get_catalog() {
        if entry.id == "_G" {
            // Base library registers directly into the global table
            (entry.open)(l, gc);
        } else {
            // Other libraries: create a table, register functions, set as global
            open_library(l, gc, entry);
        }
    }
}

/// 打开一个命名空间库（创建库表 + 注册函数 + 设置全局变量）
fn open_library(l: &mut LuaState, gc: &mut GarbageCollector, entry: &LibEntry) {
    let Some(global) = l.global_table else {
        return;
    };
    crate::registration::publish_new_table(l, gc, global, entry.name.as_bytes())
        .expect("global library Table publication must remain collector-valid");

    // The published global edge owns the table while its open function
    // installs the library's remaining entries.
    (entry.open)(l, gc);
}

/// 在指定的受管表中注册一个 C 函数。
pub fn register_in_table(
    state: &LuaState,
    gc: &mut GarbageCollector,
    table: GcRef<Table>,
    name: &str,
    function: CFunction,
) {
    crate::registration::register_c_function(state, gc, table, name.as_bytes(), function, None)
        .expect("library Function publication must remain collector-valid");
}

/// 在全局表中注册函数（用于 base 库）
pub fn register_global(
    state: &LuaState,
    gc: &mut GarbageCollector,
    global_table: GcRef<Table>,
    name: &str,
    function: CFunction,
) {
    register_in_table(state, gc, global_table, name, function);
}
