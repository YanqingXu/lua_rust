//! 标准库目录与注册 (Library Catalog)
//!
//! 管理 Lua 5.1 全部标准库的注册和查询。
//!

use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc_string::GcString;
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
    // Create the library table
    let lib_table = gc.create(Table::new());

    // Store it temporarily in a well-known location
    // We use a helper pattern: push the table ref, open the library (which
    // registers functions into it), then set as global
    let lib_ref = lib_table;
    let name_str = gc.create(GcString::new(entry.name));

    // Set empty table as global first, so open function can find it
    if let Some(gt) = l.global_table {
        let gt_ptr = gt.as_ptr() as *mut Table;
        // SAFETY: gt is a GC root
        unsafe {
            (*gt_ptr).set(&Value::String(name_str), &Value::Table(lib_ref));
        }
    }

    // Now open the library (it will register into the lib table via the global)
    (entry.open)(l, gc);
}

/// 在指定表中注册一个 C 函数（直接操作，需要 GC）
pub fn register_in_table(
    gc: &mut GarbageCollector,
    table: &mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    table.set(&Value::String(name_str), &Value::Function(func_obj));
}

/// 在全局表中注册函数（用于 base 库）
///
/// # Safety
/// `global_table` must point to a valid GC-rooted Table.
pub unsafe fn register_global(
    gc: &mut GarbageCollector,
    global_table: *mut Table,
    name: &str,
    func: unsafe extern "C" fn(*mut std::ffi::c_void) -> i32,
) {
    let name_str = gc.create(GcString::new(name));
    let func_obj = gc.create(Function::new_c(func));
    // SAFETY: caller guarantees global_table is a valid GC-rooted table
    unsafe {
        (*global_table).set(&Value::String(name_str), &Value::Function(func_obj));
    }
}
