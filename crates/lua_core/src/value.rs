//! Lua 动态类型系统的核心 — Value 类型
//!
//! `Value` 是 Lua 解释器中所有值的统一表示。它使用 Rust `enum`
//! 提供类型安全的动态类型系统。
//!

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::gc::gc_ref::GcRef;
use crate::gc_string::GcString;
use crate::types::{Function, LuaInteger, LuaNumber, Table, Thread, Userdata, ValueType};

// =====================================================================
// Value 枚举定义
// =====================================================================

/// Lua 5.1 值类型
///
/// 在 64 位系统上，此 enum 大小约 16 字节。
///
/// variant 顺序必须与 `ValueType` discriminant 值完全对应。
#[derive(Clone, Debug)]
pub enum Value {
    /// Lua nil 值 — discriminant 0
    Nil,
    /// Lua 布尔值 — discriminant 1
    Boolean(bool),
    /// 轻量用户数据（不受 GC 管理的 C 指针）— discriminant 2
    LightUserdata(GcRef<std::ffi::c_void>),
    /// Lua 数值（f64）— discriminant 3
    Number(LuaNumber),
    /// Lua 字符串（受 GC 管理）— discriminant 4
    String(GcRef<GcString>),
    /// Lua 表（受 GC 管理）— discriminant 5
    Table(GcRef<Table>),
    /// Lua 函数（受 GC 管理）— discriminant 6
    Function(GcRef<Function>),
    /// Lua 完整用户数据（受 GC 管理）— discriminant 7
    Userdata(GcRef<Userdata>),
    /// Lua 线程/协程（受 GC 管理）— discriminant 8
    Thread(GcRef<Thread>),
}

impl Value {
    /// 获取值的类型标签
    #[inline]
    pub fn value_type(&self) -> ValueType {
        match self {
            Value::Nil => ValueType::Nil,
            Value::Boolean(_) => ValueType::Boolean,
            Value::LightUserdata(_) => ValueType::LightUserdata,
            Value::Number(_) => ValueType::Number,
            Value::String(_) => ValueType::String,
            Value::Table(_) => ValueType::Table,
            Value::Function(_) => ValueType::Function,
            Value::Userdata(_) => ValueType::Userdata,
            Value::Thread(_) => ValueType::Thread,
        }
    }

    // ── 类型判断方法 ──────────────────────────────────────────

    #[inline]
    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }
    #[inline]
    pub fn is_boolean(&self) -> bool {
        matches!(self, Value::Boolean(_))
    }
    #[inline]
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    #[inline]
    pub fn is_light_userdata(&self) -> bool {
        matches!(self, Value::LightUserdata(_))
    }
    #[inline]
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }
    #[inline]
    pub fn is_table(&self) -> bool {
        matches!(self, Value::Table(_))
    }
    #[inline]
    pub fn is_function(&self) -> bool {
        matches!(self, Value::Function(_))
    }
    #[inline]
    pub fn is_userdata(&self) -> bool {
        matches!(self, Value::Userdata(_))
    }
    #[inline]
    pub fn is_thread(&self) -> bool {
        matches!(self, Value::Thread(_))
    }

    #[inline]
    pub fn is_collectable(&self) -> bool {
        self.is_string()
            || self.is_table()
            || self.is_function()
            || self.is_userdata()
            || self.is_thread()
    }

    // ── 值访问方法（panic 当类型不匹配）───────────────────────

    #[inline]
    pub fn as_boolean(&self) -> bool {
        match self {
            Value::Boolean(b) => *b,
            _ => panic_value("Boolean", self),
        }
    }

    #[inline]
    pub fn as_number(&self) -> LuaNumber {
        match self {
            Value::Number(n) => *n,
            _ => panic_value("Number", self),
        }
    }

    #[inline]
    pub fn as_integer(&self) -> LuaInteger {
        self.as_number() as LuaInteger
    }

    #[inline]
    pub fn as_light_userdata(&self) -> GcRef<std::ffi::c_void> {
        match self {
            Value::LightUserdata(p) => *p,
            _ => panic_value("LightUserdata", self),
        }
    }

    #[inline]
    pub fn as_string(&self) -> GcRef<GcString> {
        match self {
            Value::String(s) => *s,
            _ => panic_value("String", self),
        }
    }

    #[inline]
    pub fn as_table(&self) -> GcRef<Table> {
        match self {
            Value::Table(t) => *t,
            _ => panic_value("Table", self),
        }
    }

    #[inline]
    pub fn as_function(&self) -> GcRef<Function> {
        match self {
            Value::Function(f) => *f,
            _ => panic_value("Function", self),
        }
    }

    #[inline]
    pub fn as_userdata(&self) -> GcRef<Userdata> {
        match self {
            Value::Userdata(u) => *u,
            _ => panic_value("Userdata", self),
        }
    }

    #[inline]
    pub fn as_thread(&self) -> GcRef<Thread> {
        match self {
            Value::Thread(t) => *t,
            _ => panic_value("Thread", self),
        }
    }

    // ── 安全的值访问方法（返回 Option）───────────────────────

    #[inline]
    pub fn try_as_boolean(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_number(&self) -> Option<LuaNumber> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_integer(&self) -> Option<LuaInteger> {
        self.try_as_number().map(|n| n as LuaInteger)
    }
    #[inline]
    pub fn try_as_light_userdata(&self) -> Option<GcRef<std::ffi::c_void>> {
        match self {
            Value::LightUserdata(p) => Some(*p),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_string(&self) -> Option<GcRef<GcString>> {
        match self {
            Value::String(s) => Some(*s),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_table(&self) -> Option<GcRef<Table>> {
        match self {
            Value::Table(t) => Some(*t),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_function(&self) -> Option<GcRef<Function>> {
        match self {
            Value::Function(f) => Some(*f),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_userdata(&self) -> Option<GcRef<Userdata>> {
        match self {
            Value::Userdata(u) => Some(*u),
            _ => None,
        }
    }
    #[inline]
    pub fn try_as_thread(&self) -> Option<GcRef<Thread>> {
        match self {
            Value::Thread(t) => Some(*t),
            _ => None,
        }
    }

    // ── Lua 语义真值判断 ──────────────────────────────────────

    #[inline]
    pub fn is_false(&self) -> bool {
        matches!(self, Value::Nil | Value::Boolean(false))
    }

    #[inline]
    pub fn is_true(&self) -> bool {
        !self.is_false()
    }
}

// =====================================================================
// 内部辅助
// =====================================================================

#[cold]
#[track_caller]
fn panic_value(expected: &str, value: &Value) -> ! {
    panic!(
        "Value::as_{}: value is not {}, got {:?}",
        expected.to_lowercase(),
        expected,
        value.value_type()
    )
}

// =====================================================================
// PartialEq
// =====================================================================

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        if self.value_type() != other.value_type() {
            return false;
        }

        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Boolean(a), Value::Boolean(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a.to_bits() == b.to_bits(),
            // GcRef<T> 的 PartialEq 比较指针相等性
            (Value::LightUserdata(a), Value::LightUserdata(b)) => a == b,
            // Lua strings compare by byte content. Interning keeps this fast in
            // the common path, but equality must still be correct if a caller
            // creates two GC strings outside the shared pool.
            (Value::String(a), Value::String(b)) => {
                if a == b {
                    true
                } else {
                    // SAFETY: string Value operands are live while equality is evaluated.
                    let a_data = unsafe { a.as_ref() }.map(|s| s.data());
                    // SAFETY: string Value operands are live while equality is evaluated.
                    let b_data = unsafe { b.as_ref() }.map(|s| s.data());
                    a_data == b_data
                }
            }
            (Value::Table(a), Value::Table(b)) => a == b,
            (Value::Function(a), Value::Function(b)) => a == b,
            (Value::Userdata(a), Value::Userdata(b)) => a == b,
            (Value::Thread(a), Value::Thread(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

// =====================================================================
// Hash（与 Lua 值相等语义配套）
// =====================================================================

impl Hash for Value {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Value::Nil => 0_usize.hash(state),
            Value::Boolean(b) => {
                if *b {
                    1_usize.hash(state);
                } else {
                    0_usize.hash(state);
                }
            }
            Value::Number(n) => n.to_bits().hash(state),
            Value::LightUserdata(p) => p.hash(state),
            Value::String(s) => {
                // SAFETY: GC single-threaded model ensures the GcString
                // is alive while we hold a GcRef to it. Reading the
                // precomputed hash is a pure read with no side effects.
                let h = unsafe { s.as_ref() }.map(|gs| gs.hash()).unwrap_or(0);
                h.hash(state);
            }
            Value::Table(t) => t.hash(state),
            Value::Function(f) => f.hash(state),
            Value::Userdata(u) => u.hash(state),
            Value::Thread(t) => t.hash(state),
        }
    }
}

// =====================================================================
// Display（用于调试与错误信息）
// =====================================================================

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Boolean(b) => write!(f, "{}", if *b { "true" } else { "false" }),
            Value::Number(n) => write!(f, "{:.6}", n),
            Value::LightUserdata(p) => write!(f, "lightuserdata: {:p}", p.as_ptr()),
            Value::String(p) => write!(f, "string: {:p}", p.as_ptr()),
            Value::Table(p) => write!(f, "table: {:p}", p.as_ptr()),
            Value::Function(p) => {
                // SAFETY: Display takes &self, preventing concurrent GC mutation.
                // The GcRef is valid as long as no GC sweep has freed the object.
                // The single-threaded GC model guarantees this during Display.
                if let Some(func) = unsafe { p.as_ref() } {
                    if func.is_c_function() {
                        write!(f, "C function: {:p}", p.as_ptr())
                    } else {
                        write!(f, "Lua function: {:p}", p.as_ptr())
                    }
                } else {
                    write!(f, "function: {:p}", p.as_ptr())
                }
            }
            Value::Userdata(p) => write!(f, "userdata: {:p}", p.as_ptr()),
            Value::Thread(p) => write!(f, "thread: {:p}", p.as_ptr()),
        }
    }
}

// =====================================================================
// 编译期验证
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc_string::GcString;
    use std::mem;

    #[test]
    fn test_value_discriminant_matches_value_type() {
        assert_eq!(Value::Nil.value_type(), ValueType::Nil);
        assert_eq!(Value::Boolean(true).value_type(), ValueType::Boolean);
        assert_eq!(Value::Number(0.0).value_type(), ValueType::Number);
        assert_eq!(Value::String(GcRef::null()).value_type(), ValueType::String);
    }

    #[test]
    fn test_value_size_constraint() {
        let size = mem::size_of::<Value>();
        assert!(size <= 16, "Value size {} exceeds 16 bytes", size);
    }

    #[test]
    fn test_string_values_compare_by_content_without_interning() {
        let mut gc = GarbageCollector::new();
        let left = gc.create(GcString::new("same"));
        let right = gc.create(GcString::new("same"));

        assert_ne!(left, right);
        assert_eq!(Value::String(left), Value::String(right));
    }
}
