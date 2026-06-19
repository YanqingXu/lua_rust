//! Lua 函数对象（闭包）— C 函数闭包和 Lua 函数闭包
//!
//! `Function` 是 Lua 中的可调用对象，可以是 C 函数或 Lua 函数。
//! 在 Lua 中也称为 Closure（闭包）。
//!
//! ## 两种闭包类型
//! - **C 函数闭包** (`is_c = true`)：包装 C 函数指针，可以有上值
//! - **Lua 函数闭包** (`is_c = false`)：包含函数原型（Proto），可以有上值
//!
//! ## 核心字段
//! - 上值数组 (`upvalues`)：闭包捕获的外部变量
//! - 环境表 (`env`)：控制函数的全局变量访问范围（Lua 5.1 的 setfenv/getfenv）
//! - Proto 引用 / C 函数指针
//!
//! C++ 参考: `lua_cpp/src/core/function.hpp`, `lua_cpp/src/core/function.cpp`
//! 中的 `Lua::Function` 类。

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::proto::Proto;
use crate::table::Table;
use crate::types::GcObjectType;
use crate::upvalue::Upvalue;

// =====================================================================
// CFunction 类型定义
// =====================================================================

/// C 函数类型定义
///
/// C 函数接受 LuaState 指针作为参数，返回结果数量。
/// Phase 1.4 使用不透明指针（`*mut c_void`）占位，
/// Phase 3 实现 LuaState 后具体化为 `*mut LuaState`。
///
/// C++ 对应: `Lua::CFunction` = `i32 (*)(LuaState* L)`
pub type CFunction = unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;

// =====================================================================
// Function 结构体
// =====================================================================

/// Lua 函数对象（闭包）
///
/// Function 是 Lua 中的可调用对象，可以是 C 函数闭包或 Lua 函数闭包。
/// GC 管理，支持上值捕获和环境表设置。
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - is_c: bool (1 byte)
/// - nupvalues: u8 (1 byte)
/// - padding: (6 bytes)
/// - gclist: Option<GcRef<Function>> (8 bytes)
/// - env: Option<GcRef<Table>> (8 bytes)
/// - c_function: Option<CFunction> (8 bytes)
/// - proto: Option<GcRef<Proto>> (8 bytes)
/// - upvalues: Vec<GcRef<Upvalue>> (24 bytes)
///
/// C++ 对应: `Lua::Function`（继承 `GCObject`）
#[repr(C)]
pub struct Function {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    // ── ClosureHeader 字段 ──────────────────────────────────────────
    /// 是否为 C 函数
    is_c: bool,

    /// 上值数量（ClosureHeader 字段，与 upvalues.len() 保持同步）
    nupvalues: u8,

    /// GC 链表指针：用于增量 GC 和分代 GC 的灰色对象链表遍历
    gclist: Option<GcRef<Function>>,

    /// 环境表：用于控制函数的全局变量访问范围
    /// 如果为 None，则使用 LuaState 的全局表
    env: Option<GcRef<Table>>,

    // ── 函数特有字段 ────────────────────────────────────────────────
    /// C 函数指针（仅当 is_c 为 true 时有效）
    /// 对应 CClosure 的 `lua_CFunction f` 字段
    c_function: Option<CFunction>,

    /// 函数原型（仅当 is_c 为 false 时有效）
    /// 对应 LClosure 的 `struct Proto *p` 字段
    proto: Option<GcRef<Proto>>,

    /// Upvalue 数组（闭包捕获的外部变量）
    /// - C 函数（CClosure）：存储 Upvalue* 指针
    /// - Lua 函数（LClosure）：存储 Upvalue* 指针
    upvalues: Vec<GcRef<Upvalue>>,
}

impl Function {
    /// 创建 C 函数闭包
    ///
    /// C++ 对应: `Function::Function(CFunction func)`
    ///
    /// 不同于 C++ 需要 null 检查，Rust 的类型系统保证函数指针始终有效。
    pub fn new_c(func: CFunction) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Function),
            is_c: true,
            nupvalues: 0,
            gclist: None,
            env: None,
            c_function: Some(func),
            proto: None,
            upvalues: Vec::new(),
        }
    }

    /// 创建 Lua 函数闭包
    ///
    /// C++ 对应: `Function::Function(Proto* proto)`
    ///
    /// # Panics
    /// 如果 `proto` 为 null 则 panic。
    pub fn new_lua(proto: GcRef<Proto>) -> Self {
        assert!(!proto.is_null(), "Proto pointer cannot be null");
        Self {
            header: GcObjectHeader::new(GcObjectType::Function),
            is_c: false,
            nupvalues: 0,
            gclist: None,
            env: None,
            c_function: None,
            proto: Some(proto),
            upvalues: Vec::new(),
        }
    }

    // ── 类型检查 ──────────────────────────────────────────────────

    /// 是否为 C 函数
    #[inline]
    pub fn is_c_function(&self) -> bool {
        self.is_c
    }

    /// 是否为 Lua 函数
    #[inline]
    pub fn is_lua_function(&self) -> bool {
        !self.is_c
    }

    // ── C 函数访问 ────────────────────────────────────────────────

    /// 获取 C 函数指针
    ///
    /// C++ 对应: `Function::getCFunction() const`
    #[inline]
    pub fn c_function(&self) -> Option<CFunction> {
        self.c_function
    }

    // ── Lua 函数访问 ──────────────────────────────────────────────

    /// 获取函数原型（仅 Lua 函数有效）
    ///
    /// C++ 对应: `Function::getProto() const`
    #[inline]
    pub fn proto(&self) -> Option<GcRef<Proto>> {
        self.proto
    }

    // ── Upvalue 管理 ──────────────────────────────────────────────

    /// 获取 Upvalue 数量
    ///
    /// C++ 对应: `Function::getUpvalueCount() const`
    #[inline]
    pub fn upvalue_count(&self) -> usize {
        self.upvalues.len()
    }

    /// 获取指定索引的 Upvalue
    ///
    /// C++ 对应: `Function::getUpvalue(usize index) const`
    #[inline]
    pub fn upvalue(&self, index: usize) -> Option<GcRef<Upvalue>> {
        self.upvalues.get(index).copied()
    }

    /// 设置指定索引的 Upvalue
    ///
    /// # Panics
    /// 如果 index 超出范围则 panic。
    ///
    /// C++ 对应: `Function::setUpvalue(usize index, Upvalue* upvalue)`
    pub fn set_upvalue(&mut self, index: usize, upvalue: GcRef<Upvalue>) {
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, upvalue)
        self.upvalues[index] = upvalue;
    }

    /// 添加 Upvalue 到数组末尾
    ///
    /// C++ 对应: `Function::addUpvalue(Upvalue* upvalue)`
    pub fn add_upvalue(&mut self, upvalue: GcRef<Upvalue>) {
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, upvalue)
        self.upvalues.push(upvalue);
        // 同步 ClosureHeader 字段
        self.nupvalues = self.upvalues.len() as u8;
    }

    // ── 环境表管理 ────────────────────────────────────────────────

    /// 获取函数的环境表
    ///
    /// C++ 对应: `Function::getEnv() const`
    #[inline]
    pub fn env(&self) -> Option<GcRef<Table>> {
        self.env
    }

    /// 设置函数的环境表
    ///
    /// C++ 对应: `Function::setEnv(Table* env)`
    pub fn set_env(&mut self, env: Option<GcRef<Table>>) {
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, env)
        self.env = env;
    }

    // ── ClosureHeader 字段访问 ────────────────────────────────────

    /// 获取上值数量（ClosureHeader 字段）
    ///
    /// C++ 对应: `Function::getNumUpvalues() const`
    #[inline]
    pub fn num_upvalues(&self) -> u8 {
        self.nupvalues
    }

    /// 获取 GC 链表指针（ClosureHeader 字段）
    ///
    /// C++ 对应: `Function::getGCList() const`
    #[inline]
    pub fn gc_list(&self) -> Option<GcRef<Function>> {
        self.gclist
    }

    /// 设置 GC 链表指针（ClosureHeader 字段）
    ///
    /// C++ 对应: `Function::setGCList(GCObject* list)`
    #[inline]
    pub fn set_gc_list(&mut self, list: Option<GcRef<Function>>) {
        self.gclist = list;
    }
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: Function 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::Function。
// mark_children 完整标记 proto、所有 upvalue 和环境表。
unsafe impl GcObject for Function {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    /// 标记 Function 引用的所有 GC 对象
    ///
    /// 标记路径：
    /// 1. 如果是 Lua 函数，标记函数原型（Proto）
    /// 2. 标记所有上值（Upvalue）
    /// 3. 标记环境表（Table）
    ///
    /// C++ 对应: `Function::mark(GarbageCollector& gc)`
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        // 1. 标记函数原型（仅 Lua 函数）
        if let Some(proto_ref) = self.proto {
            // SAFETY: proto_ref is a valid GcRef<Proto> held by this Function;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(proto_ref.as_ptr() as *mut GcObjectHeader);
            }
        }

        // 2. 标记所有上值
        for uv in &self.upvalues {
            // SAFETY: uv is a valid GcRef<Upvalue> held by this Function;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(uv.as_ptr() as *mut GcObjectHeader);
            }
        }

        // 3. 标记环境表
        if let Some(env_ref) = self.env {
            // SAFETY: env_ref is a valid GcRef<Table> held by this Function;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(env_ref.as_ptr() as *mut GcObjectHeader);
            }
        }
    }

    fn get_size(&self) -> usize {
        // 基础大小 + upvalue 数组容量
        std::mem::size_of::<Self>()
            + self.upvalues.capacity() * std::mem::size_of::<GcRef<Upvalue>>()
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Function {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_c {
            f.debug_struct("Function")
                .field("type", &"C")
                .field("upvalues", &self.upvalues.len())
                .field("env", &self.env.is_some())
                .finish()
        } else {
            f.debug_struct("Function")
                .field("type", &"Lua")
                .field("proto", &self.proto.map(|p| p.as_ptr()))
                .field("upvalues", &self.upvalues.len())
                .field("env", &self.env.is_some())
                .finish()
        }
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::gc_ref::GcRef;
    use crate::string_pool::StringPool;
    use crate::value::Value;

    // A helper dummy C function for testing
    unsafe extern "C" fn dummy_c_func(_state: *mut std::ffi::c_void) -> i32 {
        0
    }

    // ── 创建测试 ──────────────────────────────────────────────────

    #[test]
    fn test_new_c_function() {
        let f = Function::new_c(dummy_c_func);

        assert!(f.is_c_function());
        assert!(!f.is_lua_function());
        assert!(f.c_function().is_some());
        assert!(f.proto().is_none());
        assert_eq!(f.upvalue_count(), 0);
        assert!(f.env().is_none());
    }

    #[test]
    fn test_new_lua_function() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let f = Function::new_lua(proto);

        assert!(!f.is_c_function());
        assert!(f.is_lua_function());
        assert!(f.c_function().is_none());
        assert_eq!(f.proto(), Some(proto));
        assert_eq!(f.upvalue_count(), 0);
        assert!(f.env().is_none());
    }

    #[test]
    #[should_panic(expected = "Proto pointer cannot be null")]
    fn test_new_lua_function_null_panics() {
        Function::new_lua(GcRef::null());
    }

    // ── Upvalue 管理 ──────────────────────────────────────────────

    #[test]
    fn test_add_and_get_upvalue() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f = Function::new_lua(proto);

        let uv1 = gc.create(Upvalue::new_closed(Value::Number(1.0)));
        let uv2 = gc.create(Upvalue::new_closed(Value::Number(2.0)));

        f.add_upvalue(uv1);
        f.add_upvalue(uv2);

        assert_eq!(f.upvalue_count(), 2);
        assert_eq!(f.num_upvalues(), 2);
        assert_eq!(f.upvalue(0), Some(uv1));
        assert_eq!(f.upvalue(1), Some(uv2));
        assert_eq!(f.upvalue(99), None);
    }

    #[test]
    fn test_set_upvalue() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f = Function::new_lua(proto);

        let uv1 = gc.create(Upvalue::new_closed(Value::Number(1.0)));
        let uv2 = gc.create(Upvalue::new_closed(Value::Number(2.0)));

        f.add_upvalue(uv1);
        f.set_upvalue(0, uv2);

        assert_eq!(f.upvalue(0), Some(uv2));
    }

    #[test]
    #[should_panic]
    fn test_set_upvalue_out_of_range_panics() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f = Function::new_lua(proto);
        let uv = gc.create(Upvalue::new_closed(Value::Nil));
        f.set_upvalue(0, uv); // upvalues 为空，索引越界
    }

    // ── 环境表管理 ────────────────────────────────────────────────

    #[test]
    fn test_env_table() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f = Function::new_lua(proto);

        assert!(f.env().is_none());

        let table = gc.create(Table::new());
        f.set_env(Some(table));
        assert_eq!(f.env(), Some(table));

        f.set_env(None);
        assert!(f.env().is_none());
    }

    #[test]
    fn test_c_function_can_have_env() {
        let mut gc = GarbageCollector::new();
        let mut f = Function::new_c(dummy_c_func);

        let table = gc.create(Table::new());
        f.set_env(Some(table));
        assert_eq!(f.env(), Some(table));
    }

    // ── ClosureHeader 字段 ────────────────────────────────────────

    #[test]
    fn test_num_upvalues_sync() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f = Function::new_lua(proto);

        assert_eq!(f.num_upvalues(), 0);

        let uv = gc.create(Upvalue::new_closed(Value::Nil));
        f.add_upvalue(uv);
        assert_eq!(f.num_upvalues(), 1);

        f.add_upvalue(uv);
        assert_eq!(f.num_upvalues(), 2);
    }

    #[test]
    fn test_gc_list() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f1 = Function::new_lua(proto);

        assert!(f1.gc_list().is_none());

        let f2 = gc.create(Function::new_lua(proto));
        f1.set_gc_list(Some(f2));
        assert_eq!(f1.gc_list(), Some(f2));
    }

    // ── GC 类型测试 ───────────────────────────────────────────────

    #[test]
    fn test_function_gc_header_type() {
        let f = Function::new_c(dummy_c_func);
        assert_eq!(f.gc_header().gc_type(), GcObjectType::Function);
    }

    #[test]
    fn test_function_gc_create_and_register() {
        let mut gc = GarbageCollector::new();
        let f = Function::new_c(dummy_c_func);
        let f_ref: GcRef<Function> = gc.create(f);

        assert!(!f_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    // ── GC 标记测试 ───────────────────────────────────────────────

    #[test]
    fn test_function_mark_lua_marks_proto() {
        let mut gc = GarbageCollector::new();

        let proto = gc.create(Proto::new());
        let f = Function::new_lua(proto);
        let f_ref = gc.create(f);

        gc.reset_marks();

        unsafe {
            let f_ptr = f_ref.as_ptr();
            (*f_ptr).mark_children(&mut gc);
        }

        // Proto 应被标记
        let proto_header = proto.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*proto_header).is_white(), "Proto should be marked");
        }
    }

    #[test]
    fn test_function_mark_upvalues() {
        let mut gc = GarbageCollector::new();

        let proto = gc.create(Proto::new());
        let uv = gc.create(Upvalue::new_closed(Value::Nil));
        let mut f = Function::new_lua(proto);
        f.add_upvalue(uv);
        let f_ref = gc.create(f);

        gc.reset_marks();

        unsafe {
            let f_ptr = f_ref.as_ptr();
            (*f_ptr).mark_children(&mut gc);
        }

        let uv_header = uv.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*uv_header).is_white(), "Upvalue should be marked");
        }
    }

    #[test]
    fn test_function_mark_env_table() {
        let mut gc = GarbageCollector::new();

        let proto = gc.create(Proto::new());
        let env_table = gc.create(Table::new());
        let mut f = Function::new_lua(proto);
        f.set_env(Some(env_table));
        let f_ref = gc.create(f);

        gc.reset_marks();

        unsafe {
            let f_ptr = f_ref.as_ptr();
            (*f_ptr).mark_children(&mut gc);
        }

        let table_header = env_table.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*table_header).is_white(), "Env table should be marked");
        }
    }

    #[test]
    fn test_function_mark_c_function_marks_no_proto() {
        let mut gc = GarbageCollector::new();

        let env_table = gc.create(Table::new());
        let mut f = Function::new_c(dummy_c_func);
        f.set_env(Some(env_table));
        let f_ref = gc.create(f);

        gc.reset_marks();

        // 标记 C 函数 — 不会 panic（没有 proto 可标记）
        unsafe {
            let f_ptr = f_ref.as_ptr();
            (*f_ptr).mark_children(&mut gc);
        }

        // 环境表应被标记
        let table_header = env_table.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(!(*table_header).is_white(), "Env table should be marked");
        }
    }

    // ── GC 回收测试 ───────────────────────────────────────────────

    #[test]
    fn test_function_swept_when_unreachable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let proto = gc.create(Proto::new());
        gc.create(Function::new_lua(proto));
        assert_eq!(gc.object_count(), 2); // Proto + Function

        gc.mark();
        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 2); // 两者都不是根
        assert_eq!(gc.object_count(), 0);
    }

    #[test]
    fn test_function_kept_when_root() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let proto = gc.create(Proto::new());
        gc.create_root(Function::new_lua(proto));
        assert_eq!(gc.object_count(), 2);

        let collected = gc.collect(&mut pool);
        // Proto 被 Function 的 mark_children 标记，Function 是根
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 2);
    }

    // ── get_size ──────────────────────────────────────────────────

    #[test]
    fn test_function_get_size() {
        let f = Function::new_c(dummy_c_func);
        let size = f.get_size();
        assert!(size >= std::mem::size_of::<Function>());
    }

    #[test]
    fn test_function_get_size_grows_with_upvalues() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut f = Function::new_lua(proto);

        let size_empty = f.get_size();

        for _ in 0..10 {
            let uv = gc.create(Upvalue::new_closed(Value::Nil));
            f.add_upvalue(uv);
        }

        let size_with_upvalues = f.get_size();
        assert!(
            size_with_upvalues > size_empty,
            "Size should increase with upvalues"
        );
    }

    // ── Debug 输出 ────────────────────────────────────────────────

    #[test]
    fn test_function_debug_c() {
        let f = Function::new_c(dummy_c_func);
        let debug_str = format!("{:?}", f);
        assert!(debug_str.contains("C"));
    }

    #[test]
    fn test_function_debug_lua() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let f = Function::new_lua(proto);
        let debug_str = format!("{:?}", f);
        assert!(debug_str.contains("Lua"));
    }

    // ── C Function pointer identity ────────────────────────────────

    #[test]
    fn test_c_function_pointer_stored_correctly() {
        let f = Function::new_c(dummy_c_func);
        assert!(f.c_function().is_some());
        assert!(f.is_c_function());
    }
}
