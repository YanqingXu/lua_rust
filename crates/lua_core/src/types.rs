//! Lua 解释器基础类型定义
//!
//! 本模块定义了 Lua 解释器中使用的所有基础类型、类型别名和前向声明。
//! 直接映射 C++ `src/common/types.hpp`。
//!
//! ## 命名约定
//! - `CamelCase` 类型 (struct/enum) → `CamelCase`
//! - `UPPER_CASE` 常量 → `UPPER_CASE`
//! - 与 C++ `Lua::*` 命名空间对应

// =====================================================================
// Lua 数值类型别名
// =====================================================================

/// Lua 数字类型（64 位双精度浮点数）
/// C++ 对应: `Lua::LuaNumber` = `f64` (double)
pub type LuaNumber = f64;

/// Lua 整数类型（64 位有符号整数）
/// C++ 对应: `Lua::LuaInteger` = `i64` (int64_t)
pub type LuaInteger = i64;

/// Lua 字节类型
/// C++ 对应: `lu_byte` = `u8` (uint8_t)
pub type LuByte = u8;

// =====================================================================
// GC 引用类型（重导出）
// =====================================================================

/// GC 管理对象的引用句柄
///
/// 在 Phase 1.2 从占位 `*const T` 替换为真实的 `GcRef<T>` 安全包装器。
///
/// C++ 对应: GC 裸指针（`GCString*`, `Table*`, 等）
pub use crate::gc::gc_ref::GcRef;

// =====================================================================
// GC 对象占位前向声明（逐步替换为真实类型）
// =====================================================================

// GcString 在 Phase 1.2 已实现，通过 crate::gc_string 导出
pub use crate::gc_string::GcString;

/// Lua 表对象（P1.4 — 已实现完整功能）
pub use crate::table::Table;

/// Lua 函数对象（P1.4 — 已实现）
///
/// 闭包对象，可以是 C 函数闭包或 Lua 函数闭包。
/// 持有上值（Upvalue）数组和环境表，GC 管理。
pub use crate::function::Function;

/// Lua 用户数据对象（P1.4 — 已实现）
///
/// GC 管理的字节缓冲区，允许将任意数据包装成 Lua 对象。
/// 支持元表绑定和可选的数据析构回调。
pub use crate::userdata::Userdata;

/// Lua 线程/协程对象（占位 — Phase 1.4 实现）
#[derive(Debug, Clone)]
pub struct Thread {
    _private: (),
}

/// Lua 函数原型对象（P1.4 — 已实现）
///
/// 包含编译后函数的完整元数据：字节码、常量表、嵌套函数原型、
/// 调试信息（行号、局部变量、上值名称）等。
pub use crate::proto::Proto;

/// Lua 上值对象（P1.4 — 已实现）
pub use crate::upvalue::Upvalue;

// =====================================================================
// Lua 类型标签枚举
// =====================================================================

/// Lua 值的类型标签
///
/// 定义了 Lua 中所有可能的值类型。这些类型对应 Lua 5.1 中的类型系统。
/// discriminant 值必须与 C++ `ValueType` 枚举完全一致。
///
/// 对应关系:
/// - Nil          -> LUA_TNIL (0)
/// - Boolean      -> LUA_TBOOLEAN (1)
/// - LightUserdata -> LUA_TLIGHTUSERDATA (2)
/// - Number       -> LUA_TNUMBER (3)
/// - String       -> LUA_TSTRING (4)
/// - Table        -> LUA_TTABLE (5)
/// - Function     -> LUA_TFUNCTION (6)
/// - Userdata     -> LUA_TUSERDATA (7)
/// - Thread       -> LUA_TTHREAD (8)
///
/// C++ 对应: `Lua::ValueType` (enum class : u8)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum ValueType {
    Nil = 0,
    Boolean = 1,
    LightUserdata = 2,
    Number = 3,
    String = 4,
    Table = 5,
    Function = 6,
    Userdata = 7,
    Thread = 8,
}

impl std::fmt::Display for ValueType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueType::Nil => write!(f, "nil"),
            ValueType::Boolean => write!(f, "boolean"),
            ValueType::LightUserdata => write!(f, "lightuserdata"),
            ValueType::Number => write!(f, "number"),
            ValueType::String => write!(f, "string"),
            ValueType::Table => write!(f, "table"),
            ValueType::Function => write!(f, "function"),
            ValueType::Userdata => write!(f, "userdata"),
            ValueType::Thread => write!(f, "thread"),
        }
    }
}

// =====================================================================
// 垃圾回收对象类型标签
// =====================================================================

/// 垃圾回收对象的类型标签
///
/// 定义了所有需要垃圾回收的对象类型，包括用户可见类型和内部类型。
/// discriminant 值必须与 C++ `GcObjectType` 枚举完全一致。
///
/// C++ 对应: `Lua::GCObjectType` (enum class : u8)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum GcObjectType {
    String = 4,
    Table = 5,
    Function = 6,
    Userdata = 7,
    Thread = 8,
    Proto = 9,
    Upval = 10,
}

// =====================================================================
// GC 三色标记颜色
// =====================================================================

/// 垃圾回收对象的颜色标记
///
/// 三色标记算法中的颜色定义：
/// - White（白色）：未访问的对象，可能被回收
/// - Gray（灰色）：已访问但未扫描其引用的对象
/// - Black（黑色）：已访问且已扫描所有引用的对象
///
/// C++ 对应: `Lua::GCColor` (enum class : u8)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GcColor {
    White = 0,
    Gray = 1,
    Black = 2,
}

// =====================================================================
// 编译期静态验证
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 ValueType discriminant 值与 C++ 定义一致
    #[test]
    fn test_value_type_discriminants() {
        assert_eq!(ValueType::Nil as u8, 0);
        assert_eq!(ValueType::Boolean as u8, 1);
        assert_eq!(ValueType::LightUserdata as u8, 2);
        assert_eq!(ValueType::Number as u8, 3);
        assert_eq!(ValueType::String as u8, 4);
        assert_eq!(ValueType::Table as u8, 5);
        assert_eq!(ValueType::Function as u8, 6);
        assert_eq!(ValueType::Userdata as u8, 7);
        assert_eq!(ValueType::Thread as u8, 8);
    }

    /// 验证 GcObjectType discriminant 值与 C++ 定义一致
    #[test]
    fn test_gc_object_type_discriminants() {
        assert_eq!(GcObjectType::String as u8, 4);
        assert_eq!(GcObjectType::Table as u8, 5);
        assert_eq!(GcObjectType::Function as u8, 6);
        assert_eq!(GcObjectType::Userdata as u8, 7);
        assert_eq!(GcObjectType::Thread as u8, 8);
        assert_eq!(GcObjectType::Proto as u8, 9);
        assert_eq!(GcObjectType::Upval as u8, 10);
    }

    /// 验证 GcColor discriminant 值
    #[test]
    fn test_gc_color_discriminants() {
        assert_eq!(GcColor::White as u8, 0);
        assert_eq!(GcColor::Gray as u8, 1);
        assert_eq!(GcColor::Black as u8, 2);
    }
}
