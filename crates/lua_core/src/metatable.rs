//! Lua 元方法系统：元表和元方法管理
//!
//! 元方法（metamethods）是 Lua 面向对象编程和操作符重载的核心机制，
//! 允许用户自定义表、用户数据和其他类型的行为。
//!
//! 系统架构：
//! - 元方法类型定义：17 种标准元方法（`TMS` 枚举）
//! - 元方法查找机制：从元表中查找指定的元方法
//! - 缓存优化：通过 `Table::flags` 标志位避免重复查找不存在的元方法
//!
//! 支持的元方法：
//! - 索引操作：`__index`, `__newindex`
//! - 算术运算：`__add`, `__sub`, `__mul`, `__div`, `__mod`, `__pow`, `__unm`
//! - 比较操作：`__eq`, `__lt`, `__le`
//! - 其他操作：`__concat`, `__len`, `__call`
//! - 特殊方法：`__gc`, `__mode`
//!

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_ref::GcRef;
use crate::string_pool::StringPool;
use crate::table::Table;
use crate::value::Value;

// =====================================================================
// 元方法类型枚举
// =====================================================================

/// 元方法类型枚举（Tag Method System）
///
/// 定义所有支持的元方法类型。枚举顺序与 Lua 5.1.5 保持一致。
/// 前 5 个（`TM_INDEX` 到 `TM_EQ`）是"快速"元方法，具有 flags 缓存优化。
///
/// discriminant 值保持稳定，便于用位图缓存缺失元方法。
///
///
/// 变体名使用 UPPER_CASE，便于和 Lua 元方法事件名区分。
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum TMS {
    // ===== 快速访问元方法（有缓存优化） =====
    /// `__index`: 控制表的索引访问行为（`table[key]`）
    TM_INDEX = 0,

    /// `__newindex`: 控制表的索引赋值行为（`table[key] = value`）
    TM_NEWINDEX = 1,

    /// `__gc`: 垃圾回收终结器
    TM_GC = 2,

    /// `__mode`: 弱引用模式（`"k"`, `"v"`, `"kv"`）
    TM_MODE = 3,

    /// `__eq`: 相等比较运算符（`==`, `~=`）
    TM_EQ = 4, // 最后一个快速访问元方法

    // ===== 算术运算元方法 =====
    /// `__add`: 加法运算符（`+`）
    TM_ADD = 5,

    /// `__sub`: 减法运算符（`-`）
    TM_SUB = 6,

    /// `__mul`: 乘法运算符（`*`）
    TM_MUL = 7,

    /// `__div`: 除法运算符（`/`）
    TM_DIV = 8,

    /// `__mod`: 取模运算符（`%`）
    TM_MOD = 9,

    /// `__pow`: 幂运算符（`^`）
    TM_POW = 10,

    /// `__unm`: 一元负号运算符（`-x`）
    TM_UNM = 11,

    // ===== 其他操作元方法 =====
    /// `__len`: 长度运算符（`#`）
    TM_LEN = 12,

    /// `__lt`: 小于比较运算符（`<`）
    TM_LT = 13,

    /// `__le`: 小于等于比较运算符（`<=`）
    TM_LE = 14,

    /// `__concat`: 字符串连接运算符（`..`）
    TM_CONCAT = 15,

    /// `__call`: 函数调用运算符（`obj(...)`）
    TM_CALL = 16,

    /// 元方法总数（哨兵值，非真实元方法）
    TM_N = 17,
}

impl TMS {
    /// 是否是"快速"元方法（具有 flags 缓存优化）
    ///
    /// 快速元方法包括 TM_INDEX(0) 到 TM_EQ(4)。
    #[inline]
    pub fn is_fast(&self) -> bool {
        *self <= TMS::TM_EQ
    }

    /// 获取元方法对应的标志位掩码（仅对快速元方法有效）
    ///
    /// 返回 `1u8 << discriminant`，用于 Table::flags 的位测试。
    #[inline]
    pub fn flag_bit(&self) -> u8 {
        debug_assert!(self.is_fast(), "flag_bit only valid for fast metamethods");
        1u8 << (*self as u8)
    }
}

// =====================================================================
// 元方法名称表
// =====================================================================

/// 元方法名称字符串数组
///
/// 按照 `TMS` 枚举顺序定义，用于在元表中查找对应的元方法函数。
/// 索引 `n` 对应 `TMS` discriminant 值为 `n` 的元方法。
///
pub const METAMETHOD_NAMES: [&str; TMS::TM_N as usize] = [
    "__index",    // TM_INDEX = 0
    "__newindex", // TM_NEWINDEX = 1
    "__gc",       // TM_GC = 2
    "__mode",     // TM_MODE = 3
    "__eq",       // TM_EQ = 4
    "__add",      // TM_ADD = 5
    "__sub",      // TM_SUB = 6
    "__mul",      // TM_MUL = 7
    "__div",      // TM_DIV = 8
    "__mod",      // TM_MOD = 9
    "__pow",      // TM_POW = 10
    "__unm",      // TM_UNM = 11
    "__len",      // TM_LEN = 12
    "__lt",       // TM_LT = 13
    "__le",       // TM_LE = 14
    "__concat",   // TM_CONCAT = 15
    "__call",     // TM_CALL = 16
];

/// 获取元方法名称字符串
///
#[inline]
pub fn metamethod_name(event: TMS) -> &'static str {
    METAMETHOD_NAMES[event as usize]
}

// =====================================================================
// 元方法查找函数
// =====================================================================

/// 从元表中查找指定的元方法
///
/// 这是元方法系统的核心查找函数。实现了标志位缓存机制，
/// 避免重复查找不存在的元方法。
///
/// 查找过程：
/// 1. 检查元表是否为 `None`
/// 2. 对快速元方法（`TM_INDEX`..`TM_EQ`），检查 `Table::flags` 缓存位
/// 3. 在元表中查找元方法名称对应的值（通过 `StringPool` 驻留名称字符串）
/// 4. 如果未找到且是快速元方法，更新 `flags` 缓存位
///
/// # Parameters
/// - `metatable`: 元表引用（`None` 表示无元表）
/// - `event`: 要查找的元方法类型
/// - `pool`: 字符串驻留池，用于驻留元方法名称
/// - `gc`: GC 实例，用于创建驻留字符串
///
/// # Safety
/// `metatable` 中的 `GcRef<Table>` 必须有效（未被 GC 回收）。
///
pub fn get_metamethod(
    metatable: Option<GcRef<Table>>,
    event: TMS,
    pool: &mut StringPool,
    gc: &mut GarbageCollector,
) -> Value {
    // 1. 检查元表是否为空
    let mt_ref = match metatable {
        Some(mt) => mt,
        None => return Value::Nil,
    };

    // 2. 对快速元方法，检查 flags 缓存
    if event.is_fast() {
        // SAFETY: mt_ref is a valid GcRef, GC won't run during this call
        // (we hold &mut GarbageCollector, preventing concurrent GC cycle)
        let flags = unsafe { &*mt_ref.as_ptr() }.flags();
        if flags & event.flag_bit() != 0 {
            // 标志位表示该元方法不存在，直接返回 nil
            return Value::Nil;
        }
    }

    // 3. 在元表中查找元方法名称对应的值
    let name = metamethod_name(event);
    let name_str = pool.intern(gc, name);
    let key = Value::String(name_str);

    // SAFETY: mt_ref is valid, and we hold &mut GarbageCollector
    // which prevents GC from running concurrently
    let result = unsafe { &*mt_ref.as_ptr() }.get(&key);

    // 4. 如果未找到且是快速元方法，更新 flags 标志位
    if result.is_nil() && event.is_fast() {
        // SAFETY: mt_ref is valid, exclusive access guaranteed by &mut GC
        // which prevents concurrent GC cycles.
        unsafe {
            let mt_ptr = mt_ref.as_ptr() as *mut Table;
            let new_flags = (*mt_ptr).flags() | event.flag_bit();
            (*mt_ptr).set_flags(new_flags);
        }
    }

    result
}

/// 快速元方法访问（带缓存优化）
///
/// 通过标志位快速判断元方法是否存在，避免不必要的表查找。
/// 只对前 5 个"快速"元方法（`TM_INDEX` 到 `TM_EQ`）有效。
///
/// # Panics
/// 如果 `event > TMS::TM_EQ` 则 panic（非快速元方法不应使用此函数）。
///
pub fn fast_metamethod(
    metatable: Option<GcRef<Table>>,
    event: TMS,
    pool: &mut StringPool,
    gc: &mut GarbageCollector,
) -> Value {
    // 快速元方法必须 <= TM_EQ
    assert!(
        event <= TMS::TM_EQ,
        "fast_metamethod: event must be <= TM_EQ, got {:?}",
        event
    );

    get_metamethod(metatable, event, pool, gc)
}

// =====================================================================
// 编译期静态验证
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gc::collector::GarbageCollector;
    use crate::gc::gc_ref::GcRef;
    use crate::string_pool::StringPool;
    use crate::table::Table;

    // ── 辅助函数 ────────────────────────────────────────────────

    /// 创建测试用 Table 并注册到 GC
    fn create_test_table(gc: &mut GarbageCollector) -> GcRef<Table> {
        gc.create(Table::new())
    }

    /// 设置元表中的元方法（给定名称和值）
    fn set_metamethod_raw(
        metatable: GcRef<Table>,
        name: &str,
        value: Value,
        pool: &mut StringPool,
        gc: &mut GarbageCollector,
    ) {
        let key_str = pool.intern(gc, name);
        let key = Value::String(key_str);
        // SAFETY: metatable is valid
        unsafe { &mut *(metatable.as_ptr() as *mut Table) }.set(&key, &value);
    }

    // ── TMS 枚举测试 ────────────────────────────────────────────

    #[test]
    fn test_tms_discriminants() {
        assert_eq!(TMS::TM_INDEX as u8, 0);
        assert_eq!(TMS::TM_NEWINDEX as u8, 1);
        assert_eq!(TMS::TM_GC as u8, 2);
        assert_eq!(TMS::TM_MODE as u8, 3);
        assert_eq!(TMS::TM_EQ as u8, 4);
        assert_eq!(TMS::TM_ADD as u8, 5);
        assert_eq!(TMS::TM_SUB as u8, 6);
        assert_eq!(TMS::TM_MUL as u8, 7);
        assert_eq!(TMS::TM_DIV as u8, 8);
        assert_eq!(TMS::TM_MOD as u8, 9);
        assert_eq!(TMS::TM_POW as u8, 10);
        assert_eq!(TMS::TM_UNM as u8, 11);
        assert_eq!(TMS::TM_LEN as u8, 12);
        assert_eq!(TMS::TM_LT as u8, 13);
        assert_eq!(TMS::TM_LE as u8, 14);
        assert_eq!(TMS::TM_CONCAT as u8, 15);
        assert_eq!(TMS::TM_CALL as u8, 16);
        assert_eq!(TMS::TM_N as u8, 17);
    }

    #[test]
    fn test_tms_is_fast() {
        assert!(TMS::TM_INDEX.is_fast());
        assert!(TMS::TM_NEWINDEX.is_fast());
        assert!(TMS::TM_GC.is_fast());
        assert!(TMS::TM_MODE.is_fast());
        assert!(TMS::TM_EQ.is_fast());

        assert!(!TMS::TM_ADD.is_fast());
        assert!(!TMS::TM_SUB.is_fast());
        assert!(!TMS::TM_LEN.is_fast());
        assert!(!TMS::TM_CALL.is_fast());
    }

    #[test]
    fn test_tms_flag_bit() {
        assert_eq!(TMS::TM_INDEX.flag_bit(), 1u8 << 0); // 0x01
        assert_eq!(TMS::TM_NEWINDEX.flag_bit(), 1u8 << 1); // 0x02
        assert_eq!(TMS::TM_GC.flag_bit(), 1u8 << 2); // 0x04
        assert_eq!(TMS::TM_MODE.flag_bit(), 1u8 << 3); // 0x08
        assert_eq!(TMS::TM_EQ.flag_bit(), 1u8 << 4); // 0x10
    }

    #[test]
    fn test_tms_total_count() {
        // TM_N should equal the number of actual metamethods
        assert_eq!(TMS::TM_N as usize, 17);
        assert_eq!(METAMETHOD_NAMES.len(), 17);
    }

    // ── 元方法名称测试 ──────────────────────────────────────────

    #[test]
    fn test_metamethod_names_match_enum() {
        assert_eq!(metamethod_name(TMS::TM_INDEX), "__index");
        assert_eq!(metamethod_name(TMS::TM_NEWINDEX), "__newindex");
        assert_eq!(metamethod_name(TMS::TM_GC), "__gc");
        assert_eq!(metamethod_name(TMS::TM_MODE), "__mode");
        assert_eq!(metamethod_name(TMS::TM_EQ), "__eq");
        assert_eq!(metamethod_name(TMS::TM_ADD), "__add");
        assert_eq!(metamethod_name(TMS::TM_SUB), "__sub");
        assert_eq!(metamethod_name(TMS::TM_MUL), "__mul");
        assert_eq!(metamethod_name(TMS::TM_DIV), "__div");
        assert_eq!(metamethod_name(TMS::TM_MOD), "__mod");
        assert_eq!(metamethod_name(TMS::TM_POW), "__pow");
        assert_eq!(metamethod_name(TMS::TM_UNM), "__unm");
        assert_eq!(metamethod_name(TMS::TM_LEN), "__len");
        assert_eq!(metamethod_name(TMS::TM_LT), "__lt");
        assert_eq!(metamethod_name(TMS::TM_LE), "__le");
        assert_eq!(metamethod_name(TMS::TM_CONCAT), "__concat");
        assert_eq!(metamethod_name(TMS::TM_CALL), "__call");
    }

    #[test]
    fn test_metamethod_name_count() {
        // Ensure every index from 0..TM_N has a corresponding name
        for i in 0..TMS::TM_N as u8 {
            let name = METAMETHOD_NAMES[i as usize];
            assert!(!name.is_empty(), "Missing name for TMS discriminant {}", i);
            assert!(
                name.starts_with("__"),
                "Name '{}' should start with __",
                name
            );
        }
    }

    // ── get_metamethod 测试 ──────────────────────────────────────

    #[test]
    fn test_get_metamethod_null_metatable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let result = get_metamethod(None, TMS::TM_INDEX, &mut pool, &mut gc);
        assert!(result.is_nil());
    }

    #[test]
    fn test_get_metamethod_found() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 创建元表
        let metatable = create_test_table(&mut gc);

        // 在元表中设置 __index 方法
        let index_val = Value::Number(42.0);
        set_metamethod_raw(metatable, "__index", index_val.clone(), &mut pool, &mut gc);

        // 查找 __index 元方法
        let result = get_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);
        assert_eq!(result, index_val);
    }

    #[test]
    fn test_get_metamethod_not_found() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 创建空元表
        let metatable = create_test_table(&mut gc);

        // 查找不存在的 __add 元方法
        let result = get_metamethod(Some(metatable), TMS::TM_ADD, &mut pool, &mut gc);
        assert!(result.is_nil());
    }

    #[test]
    fn test_get_metamethod_fast_cache() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let metatable = create_test_table(&mut gc);

        // 第一次查找 __index（不存在）— 应该设置 flags 缓存
        let result1 = get_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);
        assert!(result1.is_nil());

        // 验证 flags 已被设置
        // SAFETY: metatable is valid
        let flags_after = unsafe { &*metatable.as_ptr() }.flags();
        assert!(
            flags_after & TMS::TM_INDEX.flag_bit() != 0,
            "Cache bit should be set"
        );

        // 第二次查找 __index — 应该命中缓存（直接返回 nil，不查表）
        let result2 = get_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);
        assert!(result2.is_nil());
    }

    #[test]
    fn test_get_metamethod_present_resets_cache() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let metatable = create_test_table(&mut gc);

        // 先查找不存在的 __index → 缓存位被设置
        let _ = get_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);

        // 然后设置 __index
        set_metamethod_raw(
            metatable,
            "__index",
            Value::Number(99.0),
            &mut pool,
            &mut gc,
        );

        // 再次查找 — 由于 flags 被 Table::set 重置为 0，应该能找到
        let result = get_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);
        assert!(!result.is_nil());
        assert_eq!(result, Value::Number(99.0));
    }

    #[test]
    fn test_get_metamethod_non_fast_no_cache() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let metatable = create_test_table(&mut gc);

        // 查找不存在的 __add（非快速元方法）
        let _ = get_metamethod(Some(metatable), TMS::TM_ADD, &mut pool, &mut gc);

        // 非快速元方法不应设置 flags
        // SAFETY: metatable is valid
        let flags = unsafe { &*metatable.as_ptr() }.flags();
        assert_eq!(flags & (1u8 << (TMS::TM_ADD as u8)), 0);
    }

    #[test]
    fn test_get_metamethod_multiple_fast_events() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let metatable = create_test_table(&mut gc);

        // 查找多个不存在的快速元方法
        get_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);
        get_metamethod(Some(metatable), TMS::TM_GC, &mut pool, &mut gc);

        // 验证多个缓存位被独立设置
        // SAFETY: metatable is valid
        let flags = unsafe { &*metatable.as_ptr() }.flags();
        assert!(flags & TMS::TM_INDEX.flag_bit() != 0);
        assert!(flags & TMS::TM_GC.flag_bit() != 0);
        // TM_NEWINDEX 不应被设置
        assert!(flags & TMS::TM_NEWINDEX.flag_bit() == 0);
    }

    #[test]
    fn test_get_metamethod_arithmetic() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let metatable = create_test_table(&mut gc);

        // 设置 __add 元方法
        let add_fn = Value::Number(1.0); // placeholder（实际应为函数对象）
        set_metamethod_raw(metatable, "__add", add_fn.clone(), &mut pool, &mut gc);

        let result = get_metamethod(Some(metatable), TMS::TM_ADD, &mut pool, &mut gc);
        assert_eq!(result, add_fn);
    }

    // ── fast_metamethod 测试 ────────────────────────────────────

    #[test]
    fn test_fast_metamethod_works() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let metatable = create_test_table(&mut gc);
        let val = Value::Boolean(true);
        set_metamethod_raw(metatable, "__index", val.clone(), &mut pool, &mut gc);

        let result = fast_metamethod(Some(metatable), TMS::TM_INDEX, &mut pool, &mut gc);
        assert_eq!(result, val);
    }

    #[test]
    #[should_panic(expected = "fast_metamethod: event must be <= TM_EQ")]
    fn test_fast_metamethod_rejects_non_fast() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let metatable = create_test_table(&mut gc);

        // TM_ADD is not a fast metamethod — should panic
        fast_metamethod(Some(metatable), TMS::TM_ADD, &mut pool, &mut gc);
    }

    #[test]
    fn test_fast_metamethod_with_null_metatable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let result = fast_metamethod(None, TMS::TM_EQ, &mut pool, &mut gc);
        assert!(result.is_nil());
    }

    // ── 字符串驻留一致性测试 ────────────────────────────────────

    #[test]
    fn test_metamethod_name_interning_idempotent() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 多次驻留同一元方法名称应返回相同指针
        let s1 = pool.intern(&mut gc, "__index");
        let s2 = pool.intern(&mut gc, "__index");

        assert_eq!(s1, s2);
        assert_eq!(pool.len(), 1); // 只驻留了一份 "__index"
    }

    #[test]
    fn test_all_metamethod_names_can_be_interned() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        for i in 0..TMS::TM_N as usize {
            let name = METAMETHOD_NAMES[i];
            let s = pool.intern(&mut gc, name);
            assert!(!s.is_null(), "Failed to intern '{}'", name);
        }

        // 应该有 17 个唯一的元方法名称
        assert_eq!(pool.len(), 17);
    }

    // ── 集成测试：完整的元表操作流程 ────────────────────────────

    #[test]
    fn test_full_metatable_workflow() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        // 1. 创建一个表
        let table_ref = create_test_table(&mut gc);

        // 2. 创建一个元表
        let mt_ref = create_test_table(&mut gc);

        // 3. 设置 __index 元方法（当键不存在时调用）
        let fallback_val = Value::Number(999.0);
        set_metamethod_raw(mt_ref, "__index", fallback_val.clone(), &mut pool, &mut gc);

        // 4. 设置 __add 元方法
        let add_val = Value::Number(1.0);
        set_metamethod_raw(mt_ref, "__add", add_val.clone(), &mut pool, &mut gc);

        // 5. 将元表关联到表
        // SAFETY: table_ref is valid
        unsafe {
            let table = &mut *(table_ref.as_ptr() as *mut Table);
            table.set_metatable(Some(mt_ref));
        }

        // 6. 验证元表关联
        // SAFETY: table_ref is valid
        let actual_mt = unsafe { &*table_ref.as_ptr() }.metatable();
        assert_eq!(actual_mt, Some(mt_ref));

        // 7. 通过元表系统查找 __index
        let tm_index = get_metamethod(Some(mt_ref), TMS::TM_INDEX, &mut pool, &mut gc);
        assert_eq!(tm_index, fallback_val);

        // 8. 通过元表系统查找 __add
        let tm_add = get_metamethod(Some(mt_ref), TMS::TM_ADD, &mut pool, &mut gc);
        assert_eq!(tm_add, add_val);

        // 9. 查找不存在的元方法
        let tm_sub = get_metamethod(Some(mt_ref), TMS::TM_SUB, &mut pool, &mut gc);
        assert!(tm_sub.is_nil());
    }

    #[test]
    fn test_flags_isolated_per_metatable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let mt1 = create_test_table(&mut gc);
        let mt2 = create_test_table(&mut gc);

        // mt1: 查找 __index（不存在）→ flags 被设置
        get_metamethod(Some(mt1), TMS::TM_INDEX, &mut pool, &mut gc);

        // mt2: 不应受影响
        // SAFETY: mt2 is valid
        let mt2_flags = unsafe { &*mt2.as_ptr() }.flags();
        assert_eq!(mt2_flags, 0, "mt2 flags should be independent of mt1");
    }
}
