//! Value 系统测试套件
//!
//! 验证 Rust `Value` 实现与 C++ `Value` 类的行为等价性。
//! C++ 参考测试: `lua_cpp/tests/unit/core/test_value.cpp`

use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::types::{Function, Table, Thread, Userdata, ValueType};
use lua_core::value::Value;
use std::mem;

// ════════════════════════════════════════════════════════════════
// Discriminant 顺序验证
// ════════════════════════════════════════════════════════════════

#[test]
fn test_value_discriminant_order() {
    assert_eq!(Value::Nil.value_type(), ValueType::Nil);
    assert_eq!(Value::Boolean(true).value_type(), ValueType::Boolean);
    assert_eq!(
        Value::LightUserdata(GcRef::null()).value_type(),
        ValueType::LightUserdata
    );
    assert_eq!(Value::Number(0.0).value_type(), ValueType::Number);
    assert_eq!(Value::String(GcRef::null()).value_type(), ValueType::String);
    assert_eq!(Value::Table(GcRef::null()).value_type(), ValueType::Table);
    assert_eq!(
        Value::Function(GcRef::null()).value_type(),
        ValueType::Function
    );
    assert_eq!(
        Value::Userdata(GcRef::null()).value_type(),
        ValueType::Userdata
    );
    assert_eq!(Value::Thread(GcRef::null()).value_type(), ValueType::Thread);
}

// ════════════════════════════════════════════════════════════════
// 类型检查方法
// ════════════════════════════════════════════════════════════════

#[test]
fn test_value_nil_default() {
    let v = Value::Nil;
    assert!(v.is_nil());
    assert!(!v.is_boolean());
    assert!(!v.is_number());
    assert!(!v.is_string());
}

#[test]
fn test_value_boolean_roundtrip() {
    let v = Value::Boolean(true);
    assert!(v.is_boolean());
    assert_eq!(v.as_boolean(), true);
}

#[test]
fn test_value_number_roundtrip() {
    let v = Value::Number(3.14);
    assert!(v.is_number());
    assert!((v.as_number() - 3.14).abs() < f64::EPSILON);
}

#[test]
fn test_value_number_zero() {
    assert_eq!(Value::Number(0.0).as_number(), 0.0);
}

#[test]
fn test_value_integer_from_number() {
    assert_eq!(Value::Number(42.0).as_integer(), 42);
}

#[test]
fn test_value_integer_truncation() {
    assert_eq!(Value::Number(3.7).as_integer(), 3);
}

#[test]
fn test_value_light_userdata_roundtrip() {
    let p = unsafe { GcRef::from_ptr(0xDEAD_BEEF_usize as *const std::ffi::c_void) };
    let v = Value::LightUserdata(p);
    assert!(v.is_light_userdata());
    assert_eq!(v.as_light_userdata(), p);
}

#[test]
fn test_value_string_type_check() {
    let p = unsafe { GcRef::<GcString>::from_ptr(0x1000 as *const _) };
    let v = Value::String(p);
    assert!(v.is_string());
    assert!(!v.is_nil());
}

#[test]
fn test_value_table_type_check() {
    let p = unsafe { GcRef::<Table>::from_ptr(0x2000 as *const _) };
    assert!(Value::Table(p).is_table());
}

#[test]
fn test_value_function_type_check() {
    let p = unsafe { GcRef::<Function>::from_ptr(0x3000 as *const _) };
    assert!(Value::Function(p).is_function());
}

#[test]
fn test_value_userdata_type_check() {
    let p = unsafe { GcRef::<Userdata>::from_ptr(0x4000 as *const _) };
    assert!(Value::Userdata(p).is_userdata());
}

#[test]
fn test_value_thread_type_check() {
    let p = unsafe { GcRef::<Thread>::from_ptr(0x5000 as *const _) };
    assert!(Value::Thread(p).is_thread());
}

#[test]
fn test_value_is_collectable() {
    assert!(Value::String(GcRef::null()).is_collectable());
    assert!(Value::Table(GcRef::null()).is_collectable());
    assert!(Value::Function(GcRef::null()).is_collectable());
    assert!(Value::Userdata(GcRef::null()).is_collectable());
    assert!(Value::Thread(GcRef::null()).is_collectable());

    assert!(!Value::Nil.is_collectable());
    assert!(!Value::Boolean(true).is_collectable());
    assert!(!Value::Number(1.0).is_collectable());
}

// ════════════════════════════════════════════════════════════════
// Lua 真值语义
// ════════════════════════════════════════════════════════════════

#[test]
fn test_lua_truthiness_nil_is_falsy() {
    assert!(Value::Nil.is_false());
    assert!(!Value::Nil.is_true());
}

#[test]
fn test_lua_truthiness_false_is_falsy() {
    assert!(Value::Boolean(false).is_false());
}

#[test]
fn test_lua_truthiness_true_is_truthy() {
    assert!(Value::Boolean(true).is_true());
    assert!(!Value::Boolean(true).is_false());
}

#[test]
fn test_lua_truthiness_zero_is_truthy() {
    assert!(Value::Number(0.0).is_true());
}

#[test]
fn test_lua_truthiness_gc_string_is_truthy() {
    assert!(Value::String(GcRef::null()).is_true());
}

#[test]
fn test_lua_truthiness_light_userdata_null_is_truthy() {
    assert!(Value::LightUserdata(GcRef::null()).is_true());
}

// ════════════════════════════════════════════════════════════════
// 相等性比较
// ════════════════════════════════════════════════════════════════

#[test]
fn test_value_equality_nil() {
    assert_eq!(Value::Nil, Value::Nil);
}

#[test]
fn test_value_equality_same_boolean() {
    assert_eq!(Value::Boolean(true), Value::Boolean(true));
    assert_ne!(Value::Boolean(true), Value::Boolean(false));
}

#[test]
fn test_value_equality_same_number() {
    assert_eq!(Value::Number(1.0), Value::Number(1.0));
    assert_ne!(Value::Number(1.0), Value::Number(2.0));
}

#[test]
fn test_value_equality_different_type() {
    assert_ne!(Value::Nil, Value::Boolean(false));
    assert_ne!(Value::Boolean(true), Value::Number(1.0));
    assert_ne!(Value::Number(0.0), Value::Nil);
}

#[test]
fn test_value_equality_gc_pointer_identity() {
    let p1 = unsafe { GcRef::<GcString>::from_ptr(0x1000 as *const _) };
    let p2 = unsafe { GcRef::<GcString>::from_ptr(0x2000 as *const _) };
    assert_eq!(Value::String(p1), Value::String(p1));
    assert_ne!(Value::String(p1), Value::String(p2));
}

#[test]
fn test_value_equality_light_userdata() {
    let p1 = unsafe { GcRef::from_ptr(0xAAAA as *const std::ffi::c_void) };
    let p2 = unsafe { GcRef::from_ptr(0xBBBB as *const std::ffi::c_void) };
    assert_eq!(Value::LightUserdata(p1), Value::LightUserdata(p1));
    assert_ne!(Value::LightUserdata(p1), Value::LightUserdata(p2));
}

#[test]
fn test_value_equality_nan() {
    let nan = Value::Number(f64::NAN);
    assert_eq!(nan, nan);
}

// ════════════════════════════════════════════════════════════════
// Display / toString
// ════════════════════════════════════════════════════════════════

#[test]
fn test_display_nil() {
    assert_eq!(format!("{}", Value::Nil), "nil");
}

#[test]
fn test_display_boolean() {
    assert_eq!(format!("{}", Value::Boolean(true)), "true");
    assert_eq!(format!("{}", Value::Boolean(false)), "false");
}

#[test]
fn test_display_number() {
    assert_eq!(format!("{}", Value::Number(42.0)), "42.000000");
    assert_eq!(format!("{}", Value::Number(3.14)), "3.140000");
    assert_eq!(format!("{}", Value::Number(-1.5)), "-1.500000");
    assert_eq!(format!("{}", Value::Number(0.0)), "0.000000");
}

#[test]
fn test_display_light_userdata() {
    let p = unsafe { GcRef::<std::ffi::c_void>::from_ptr(0xDEAD_BEEF_usize as *const _) };
    let output = format!("{}", Value::LightUserdata(p));
    assert!(output.starts_with("lightuserdata: 0x"), "Got '{}'", output);
}

#[test]
fn test_display_gc_types() {
    let p = unsafe { GcRef::<GcString>::from_ptr(0x1000 as *const _) };
    assert!(format!("{}", Value::String(p)).starts_with("string: 0x"));

    let p = unsafe { GcRef::<Table>::from_ptr(0x2000 as *const _) };
    assert!(format!("{}", Value::Table(p)).starts_with("table: 0x"));

    // Function Display requires a valid GcRef to distinguish C/Lua functions,
    // so use a real object for testing.
    {
        use lua_core::function::Function;
        use lua_core::gc::collector::GarbageCollector;
        use lua_core::proto::Proto;

        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let func = gc.create(Function::new_lua(proto));
        let output = format!("{}", Value::Function(func));
        assert!(output.starts_with("Lua function: 0x"), "Got '{}'", output);
    }

    let p = unsafe { GcRef::<Userdata>::from_ptr(0x4000 as *const _) };
    assert!(format!("{}", Value::Userdata(p)).starts_with("userdata: 0x"));

    let p = unsafe { GcRef::<Thread>::from_ptr(0x5000 as *const _) };
    assert!(format!("{}", Value::Thread(p)).starts_with("thread: 0x"));
}

// ════════════════════════════════════════════════════════════════
// 安全访问器
// ════════════════════════════════════════════════════════════════

#[test]
fn test_try_as_boolean() {
    assert_eq!(Value::Boolean(true).try_as_boolean(), Some(true));
    assert_eq!(Value::Nil.try_as_boolean(), None);
}

#[test]
fn test_try_as_number() {
    assert_eq!(Value::Number(3.14).try_as_number(), Some(3.14));
    assert_eq!(Value::Nil.try_as_number(), None);
}

#[test]
fn test_try_as_integer() {
    assert_eq!(Value::Number(42.0).try_as_integer(), Some(42));
    assert_eq!(Value::Nil.try_as_integer(), None);
}

// ════════════════════════════════════════════════════════════════
// 内存大小约束
// ════════════════════════════════════════════════════════════════

#[test]
fn test_value_size_constraint() {
    let size = mem::size_of::<Value>();
    assert!(size <= 16, "Value size {} exceeds 16 bytes", size);
}

#[test]
fn test_value_size_reasonable() {
    let size = mem::size_of::<Value>();
    assert!(size >= 8, "Value size {} seems too small", size);
}

// ════════════════════════════════════════════════════════════════
// Clone 行为
// ════════════════════════════════════════════════════════════════

#[test]
fn test_value_clone() {
    assert_eq!(Value::Nil, Value::Nil.clone());
    assert_eq!(Value::Number(3.14), Value::Number(3.14).clone());
    assert_eq!(Value::Boolean(true), Value::Boolean(true).clone());
}

// ════════════════════════════════════════════════════════════════
// Panic 行为
// ════════════════════════════════════════════════════════════════

#[test]
#[should_panic]
fn test_as_boolean_panics_on_nil() {
    Value::Nil.as_boolean();
}

#[test]
#[should_panic]
fn test_as_number_panics_on_nil() {
    Value::Nil.as_number();
}

#[test]
#[should_panic]
fn test_as_string_panics_on_nil() {
    Value::Nil.as_string();
}
