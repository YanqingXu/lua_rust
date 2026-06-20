//! 标准库目录与注册 (Library Catalog)
//!
//! 管理 Lua 5.1 全部标准库的注册和查询。
//!
//! C++ 参考: `lua_cpp/src/lib/lib_catalog.hpp/.cpp`

use lua_vm::state::LuaState;

/// 库打开函数类型（C 函数签名：返回栈上返回值数量）
pub type LibOpenFn = fn(&mut LuaState) -> i32;

/// 库模块打开函数类型
pub type LibModuleOpenFn = fn(&mut LuaState);

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
pub fn open_all(l: &mut LuaState) {
    for entry in get_catalog() {
        if entry.id == "_G" {
            (entry.open)(l);
        }
    }
    for entry in get_catalog() {
        if entry.id != "_G" {
            push_lib_table(l, entry.name);
            (entry.open)(l);
            l.pop(); // pop the library table
            set_global(l, entry.name);
        }
    }
}

/// 推入库表（创建空表用于注册库函数）
fn push_lib_table(l: &mut LuaState, _name: &str) {
    l.push_nil(); // placeholder — actual table creation needs GC
}

/// 设置全局变量（将栈顶值设置为全局名）
fn set_global(_l: &mut LuaState, _name: &str) {
    // TODO: actual global set
}

/// 注册 C 函数到当前栈顶表
pub fn register_function(l: &mut LuaState, _name: &str, _func: LibOpenFn) {
    l.push_nil(); // placeholder: push function
    l.push_nil(); // placeholder: push function
    // TODO: lua_setfield or equivalent
}

/// 注册库表（推入以 name 命名的空表）
pub fn register_lib_table(l: &mut LuaState, _name: &str) {
    l.push_nil(); // placeholder: push new table
}
