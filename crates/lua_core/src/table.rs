//! Lua 表系统：混合数组/哈希表实现
//!
//! `Table` 是 Lua 最重要的数据结构，同时具备数组和哈希表的特性。
//! 内部采用混合存储策略：
//! - **数组部分**：存储连续的正整数键（1, 2, 3, ...），使用 `Vec<Value>` 实现 O(1) 访问
//! - **哈希部分**：存储其他类型的键或非连续的整数键，使用 `HashMap<Value, Value>` 实现
//!
//! C++ 参考: `lua_cpp/src/core/table.hpp`, `lua_cpp/src/core/table.cpp`

use std::collections::HashMap;

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::types::{GcObjectType, LuaNumber};
use crate::value::Value;

// =====================================================================
// 常量
// =====================================================================

/// 数组索引的最大有效范围（对齐 C++ isArrayIndex 的限制）
/// 防止过大的索引导致内存问题
const MAX_ARRAY_INDEX: i32 = 1_000_000;

// =====================================================================
// Table 结构体
// =====================================================================

/// Lua 表对象
///
/// 实现 Lua 的表数据结构。GC 管理，支持数组部分（连续整数键）
/// 和哈希部分（其他键）的混合存储。
///
/// 内存布局（`#[repr(C)]`，header 必须在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - array: `Vec<Value>` (24 bytes)
/// - hash: `HashMap<Value, Value>` (~56 bytes)
/// - metatable: `Option<GcRef<Table>>` (8 bytes)
/// - flags: u8 (1 byte)
///   总计约 105+ bytes
///
/// C++ 对应: `Lua::Table`
#[repr(C)]
pub struct Table {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    /// 数组部分：存储连续的正整数键（索引从 1 开始）
    array: Vec<Value>,

    /// 哈希部分：存储其他类型的键或非连续的整数键
    hash: HashMap<Value, Value>,
    /// 哈希键的稳定迭代顺序。删除会留下 nil 墓碑，避免 `next`
    /// 在遍历期间删除当前键时退化或失去位置。
    hash_order: Vec<Value>,
    /// 哈希键到 `hash_order` 位置的索引。
    hash_positions: HashMap<Value, usize>,

    /// 元表指针（None 表示无元表）
    metatable: Option<GcRef<Table>>,

    /// 元方法缓存标志位
    /// 每个位对应一个元方法类型，位为 1 表示该元方法不存在
    flags: u8,
}

impl Table {
    /// 创建空表
    ///
    /// C++ 对应: `Table::Table()`
    pub fn new() -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Table),
            array: Vec::new(),
            hash: HashMap::new(),
            hash_order: Vec::new(),
            hash_positions: HashMap::new(),
            metatable: None,
            flags: 0,
        }
    }

    /// 创建具有预分配容量的空表
    ///
    /// `array_capacity`: 数组部分的初始容量
    /// `hash_capacity`: 哈希部分的初始容量
    pub fn with_capacity(array_capacity: usize, hash_capacity: usize) -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Table),
            array: Vec::with_capacity(array_capacity),
            hash: HashMap::with_capacity(hash_capacity),
            hash_order: Vec::with_capacity(hash_capacity),
            hash_positions: HashMap::with_capacity(hash_capacity),
            metatable: None,
            flags: 0,
        }
    }

    // =====================================================================
    // 基本操作
    // =====================================================================

    /// 获取键对应的值
    ///
    /// 查找策略：
    /// 1. 如果 key 是正整数且在数组范围内，从数组部分获取
    /// 2. 否则从哈希部分获取
    /// 3. 如果键不存在，返回 nil
    ///
    /// C++ 对应: `Table::get(const Value& key)`
    pub fn get(&self, key: &Value) -> Value {
        let mut index: i32 = 0;
        if Self::is_array_index(key, &mut index) {
            return self.get_array(index);
        }

        self.hash.get(key).cloned().unwrap_or(Value::Nil)
    }

    /// 设置键值对
    ///
    /// 设置策略：
    /// 1. 如果 key 是 nil，触发 panic（Lua 语义：nil 键不允许）
    /// 2. 如果 key 是 NaN，触发 panic（Lua 语义：NaN 键不允许）
    /// 3. 如果 value 是 nil，删除该键
    /// 4. 如果 key 是正整数且较小，存储到数组部分
    /// 5. 否则存储到哈希部分
    ///
    /// C++ 对应: `Table::set(const Value& key, const Value& value)`
    pub fn set(&mut self, key: &Value, value: &Value) {
        // Lua 语义：nil 键不允许
        if key.is_nil() {
            panic!("table index is nil");
        }
        if key.is_number() {
            let n = key.as_number();
            if n.is_nan() {
                panic!("table index is NaN");
            }
        }

        self.flags = 0;

        // 如果 value 是 nil，表示删除该键
        if value.is_nil() {
            self.remove(key);
            return;
        }

        // 检查是否是数组索引
        let mut index: i32 = 0;
        if Self::is_array_index(key, &mut index) {
            self.set_array(index, value);
            return;
        }

        // TODO Phase 1.3: 写屏障 — gc->writeBarrier(this, key) / gc->writeBarrier(this, value)
        // 当前 Phase 1.2 GC 骨架尚未实现 writeBarrier，后续阶段补全。

        // 存储到哈希部分。Value 的 Eq/Hash 已按字符串内容处理，因此即使
        // 调用方没有使用同一个 StringPool 驻留，同内容字符串也会命中同一键。
        let hash_key = key.clone();
        if !self.hash.contains_key(&hash_key) {
            self.hash_positions
                .insert(hash_key.clone(), self.hash_order.len());
            self.hash_order.push(hash_key.clone());
        }
        self.hash.insert(hash_key, value.clone());
    }

    /// 检查键是否存在且值不为 nil
    ///
    /// C++ 对应: `Table::has(const Value& key)`
    pub fn has(&self, key: &Value) -> bool {
        let mut index: i32 = 0;
        if Self::is_array_index(key, &mut index) {
            if index >= 1 && (index as usize) <= self.array.len() {
                return !self.array[(index - 1) as usize].is_nil();
            }
            return false;
        }

        self.hash.get(key).is_some_and(|v| !v.is_nil())
    }

    /// 删除键值对
    ///
    /// 等价于 `set(key, &Value::Nil)`
    ///
    /// C++ 对应: `Table::remove(const Value& key)`
    pub fn remove(&mut self, key: &Value) {
        self.flags = 0;

        let mut index: i32 = 0;
        if Self::is_array_index(key, &mut index) {
            if index >= 1 && (index as usize) <= self.array.len() {
                self.array[(index - 1) as usize] = Value::Nil;
            }
            return;
        }

        let removed_key = self
            .hash
            .remove_entry(key)
            .map(|(removed_key, _)| removed_key);

        if let Some(removed_key) = removed_key
            && let Some(pos) = self.hash_positions.remove(&removed_key)
            && let Some(slot) = self.hash_order.get_mut(pos)
        {
            *slot = Value::Nil;
        }
    }

    /// 清空表内容和元表引用
    ///
    /// 主要用于测试/关闭阶段重置固定 registry 等全局表，
    /// 避免其中保留已被 GC 清除的对象指针。
    ///
    /// C++ 对应: `Table::clear()`
    pub fn clear(&mut self) {
        self.array.clear();
        self.hash.clear();
        self.hash_order.clear();
        self.hash_positions.clear();
        self.metatable = None;
        self.flags = 0;
    }

    // =====================================================================
    // 数组操作
    // =====================================================================

    /// 获取数组元素
    ///
    /// Lua 数组使用 1-based 索引，即第一个元素的索引是 1。
    ///
    /// C++ 对应: `Table::getArray(i32 index)`
    pub fn get_array(&self, index: i32) -> Value {
        // Lua 数组是 1-based
        if index < 1 {
            return Value::Nil;
        }

        let array_index = (index - 1) as usize;
        if array_index < self.array.len() {
            return self.array[array_index].clone();
        }

        // 索引超出范围，返回 nil
        Value::Nil
    }

    /// 设置数组元素
    ///
    /// Lua 数组使用 1-based 索引。如果索引超出当前数组大小，
    /// 会自动扩展数组（中间的空位填充 nil）。
    ///
    /// C++ 对应: `Table::setArray(i32 index, const Value& value)`
    pub fn set_array(&mut self, index: i32, value: &Value) {
        // Lua 数组是 1-based
        if index < 1 {
            // C++ 同样忽略无效索引
            return;
        }

        self.flags = 0;

        let array_index = (index - 1) as usize;

        // 如果索引超出当前大小，扩展数组
        if array_index >= self.array.len() {
            // 扩展数组，中间的空位填充 nil
            self.array.resize(array_index + 1, Value::Nil);
        }

        // TODO Phase 1.3: 写屏障 — gc->writeBarrier(this, value)

        self.array[array_index] = value.clone();
    }

    /// 获取数组部分的大小
    ///
    /// C++ 对应: `Table::getArraySize()`
    #[inline]
    pub fn array_size(&self) -> usize {
        self.array.len()
    }

    /// 获取表的长度（Lua 的 `#` 运算符）
    ///
    /// 返回数组部分中最后一个非 nil 值的索引。
    /// 这是 Lua 5.1 长度语义的完整二分搜索实现。
    ///
    /// C++ 对应: `Table::length()`
    pub fn length(&self) -> usize {
        let array_size = self.array.len();

        // 辅助闭包：检查给定整数键是否有非 nil 值
        let has_integer_key = |this: &Self, index: usize| -> bool {
            if index == 0 || index > i32::MAX as usize {
                return false;
            }

            let key = Value::Number(index as LuaNumber);
            let mut arr_idx: i32 = 0;
            if Self::is_array_index(&key, &mut arr_idx) {
                let uidx = arr_idx as usize;
                return uidx <= this.array.len() && !this.array[uidx - 1].is_nil();
            }

            this.hash.get(&key).is_some_and(|v| !v.is_nil())
        };

        if array_size > 0 {
            // 如果最后一个数组元素是 nil，在数组内二分查找边界
            if self.array[array_size - 1].is_nil() {
                let mut low: usize = 0;
                let mut high: usize = array_size;
                while high - low > 1 {
                    let mid = low + (high - low) / 2;
                    if self.array[mid - 1].is_nil() {
                        high = mid;
                    } else {
                        low = mid;
                    }
                }
                return low;
            }

            // 最后一个元素非 nil，检查是否在数组外还有边界
            if !has_integer_key(self, array_size + 1) {
                return array_size;
            }
        }

        // 在数组外二分搜索边界
        let mut low = array_size;
        let mut high = if array_size == 0 { 1 } else { array_size * 2 };

        // 指数增长找到上界
        while has_integer_key(self, high) {
            low = high;
            if high > (i32::MAX as usize) / 2 {
                return high;
            }
            high *= 2;
        }

        // 二分搜索精确边界
        while high - low > 1 {
            let mid = low + (high - low) / 2;
            if has_integer_key(self, mid) {
                low = mid;
            } else {
                high = mid;
            }
        }

        low
    }

    // =====================================================================
    // 迭代器支持
    // =====================================================================

    /// 获取表中的下一个键值对（用于泛型 for 循环）
    ///
    /// 这是实现 Lua 的 `next()` 函数和 `pairs()` 迭代器的核心方法。
    /// 遍历顺序：先遍历数组部分（索引 1, 2, 3, ...），再遍历哈希部分。
    ///
    /// 返回 `Some((next_key, next_value))` 如果找到下一个键值对，
    /// 返回 `None` 如果已到表尾。
    ///
    /// C++ 对应: `Table::next(const Value& key, Value& nextKey, Value& nextValue)`
    pub fn next(&self, key: &Value) -> Option<(Value, Value)> {
        // 如果 key 是 nil，从头开始遍历
        if key.is_nil() {
            // 先检查数组部分
            if !self.array.is_empty() {
                // 返回第一个非 nil 的数组元素
                for (i, val) in self.array.iter().enumerate() {
                    if !val.is_nil() {
                        let next_key = Value::Number((i + 1) as LuaNumber); // Lua 索引从 1 开始
                        let next_value = val.clone();
                        return Some((next_key, next_value));
                    }
                }
            }

            // 数组部分为空或全是 nil，检查哈希部分
            return self.next_hash_from(0);

            // 表为空
        }

        // 查找当前键的位置
        let mut arr_idx: i32 = 0;
        if Self::is_array_index(key, &mut arr_idx) {
            // 当前键在数组部分 — 继续遍历数组部分
            for i in (arr_idx as usize)..self.array.len() {
                if !self.array[i].is_nil() {
                    let next_key = Value::Number((i + 1) as LuaNumber);
                    let next_value = self.array[i].clone();
                    return Some((next_key, next_value));
                }
            }

            // 数组部分遍历完毕，转到哈希部分
            return self.next_hash_from(0);
        }

        // 当前键在哈希部分
        // Lua 5.1 允许遍历过程中删除当前键，此时从任意剩余哈希条目继续
        if !self.hash.contains_key(key) {
            return self.next_hash_from(0);
        }

        let next_pos = self.hash_positions.get(key).map(|pos| pos + 1).unwrap_or(0);
        self.next_hash_from(next_pos)
    }

    fn next_hash_from(&self, start: usize) -> Option<(Value, Value)> {
        for key in self.hash_order.iter().skip(start) {
            if key.is_nil() {
                continue;
            }
            if let Some(value) = self.hash.get(key)
                && !value.is_nil()
            {
                return Some((key.clone(), value.clone()));
            }
        }
        None
    }

    // =====================================================================
    // 元表操作
    // =====================================================================

    /// 获取元表
    ///
    /// C++ 对应: `Table::getMetatable()`
    #[inline]
    pub fn metatable(&self) -> Option<GcRef<Table>> {
        self.metatable
    }

    /// 设置元表
    ///
    /// C++ 对应: `Table::setMetatable(Table* mt)`
    pub fn set_metatable(&mut self, mt: Option<GcRef<Table>>) {
        // TODO Phase 1.3: 写屏障 — gc->writeBarrier(this, mt)
        self.metatable = mt;
    }

    /// 获取元方法缓存标志位
    ///
    /// C++ 对应: `Table::getFlags()`
    #[inline]
    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// 设置元方法缓存标志位
    ///
    /// C++ 对应: `Table::setFlags(u8 flags)`
    #[inline]
    pub fn set_flags(&mut self, flags: u8) {
        self.flags = flags;
    }

    // =====================================================================
    // 调试和统计
    // =====================================================================

    /// 获取哈希部分的大小
    ///
    /// C++ 对应: `Table::getHashSize()`
    #[inline]
    pub fn hash_size(&self) -> usize {
        self.hash.len()
    }

    /// 返回哈希部分的所有条目迭代器
    ///
    /// 用于需要按内容查找字符串键的场景（字符串驻留未激活时的回退路径）
    pub fn hash_entries(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.hash.iter()
    }

    /// 获取表的总元素数量
    ///
    /// C++ 对应: `Table::getTotalSize()`
    #[inline]
    pub fn total_size(&self) -> usize {
        self.array.len() + self.hash.len()
    }

    // =====================================================================
    // 内部辅助方法
    // =====================================================================

    /// 检查键是否是有效的数组索引
    ///
    /// 有效的数组索引必须满足：
    /// 1. 是数字类型
    /// 2. 是正整数
    /// 3. 在合理的范围内（1 到 MAX_ARRAY_INDEX）
    ///
    /// C++ 对应: `Table::isArrayIndex(const Value& key, i32& outIndex)`
    fn is_array_index(key: &Value, out_index: &mut i32) -> bool {
        // 必须是数字类型
        if !key.is_number() {
            return false;
        }

        let num = key.as_number();

        // 必须是正整数（非零、非负、整数）
        if num <= 0.0 || num != num.floor() {
            return false;
        }

        // 检查范围（避免过大的索引）
        if num < 1.0 || num > MAX_ARRAY_INDEX as LuaNumber {
            return false;
        }

        // 安全转换为整数
        *out_index = num as i32;
        true
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: Table 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::Table。
// mark_children 遍历数组、哈希部分和元表中的所有 GC 引用。
unsafe impl GcObject for Table {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    /// 标记表中引用的所有 GC 对象
    ///
    /// 遍历数组部分和哈希部分，标记所有引用的 GC 对象：
    /// - 字符串对象
    /// - 表对象
    /// - 函数对象
    /// - 用户数据对象
    /// - 线程对象
    /// - 元表
    ///
    /// C++ 对应: `Table::mark(GarbageCollector& gc)`
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        // 标记数组部分中的 GC 对象
        for val in &self.array {
            Self::mark_value(val, collector);
        }

        // 标记哈希部分中的 GC 对象
        for (key, val) in &self.hash {
            Self::mark_value(key, collector);
            Self::mark_value(val, collector);
        }

        // 标记元表
        if let Some(mt) = self.metatable {
            // SAFETY: mark_children is an unsafe fn, caller guarantees
            // collector is valid during the mark phase.
            let header_ptr = mt.as_ptr() as *mut GcObjectHeader;
            // SAFETY: header_ptr derives from a valid GcRef<Table>.
            unsafe {
                collector.mark_object(header_ptr);
            }
        }
    }

    /// 获取表占用的内存大小
    ///
    /// 包括：
    /// - Table 对象本身的大小
    /// - 数组部分的容量
    /// - 哈希部分的容量（估算）
    ///
    /// C++ 对应: `Table::getSize()`
    fn get_size(&self) -> usize {
        let mut size = std::mem::size_of::<Self>();

        // 数组部分的容量
        size += self.array.capacity() * std::mem::size_of::<Value>();

        // 哈希部分的容量（估算）
        // HashMap 的内存布局比较复杂，使用简化的估算
        size += self.hash.len() * (std::mem::size_of::<Value>() * 2 + std::mem::size_of::<usize>());
        size += self.hash_order.capacity() * std::mem::size_of::<Value>();
        size += self.hash_positions.len()
            * (std::mem::size_of::<Value>() + std::mem::size_of::<usize>());

        size
    }
}

impl Table {
    /// 标记单个 Value 中引用的 GC 对象
    ///
    /// 辅助方法，供 `mark_children` 和后续 `markContents` 使用。
    fn mark_value(val: &Value, collector: &mut GarbageCollector) {
        match val {
            Value::String(s) => {
                let header_ptr = s.as_ptr() as *mut GcObjectHeader;
                // SAFETY: s is a valid GC reference
                unsafe {
                    collector.mark_object(header_ptr);
                }
            }
            Value::Table(t) => {
                let header_ptr = t.as_ptr() as *mut GcObjectHeader;
                // SAFETY: t is a valid GC reference; header_ptr points to a
                // registered GcObjectHeader in the GC's intrusive list.
                unsafe {
                    collector.mark_object(header_ptr);
                }
            }
            Value::Function(f) => {
                let header_ptr = f.as_ptr() as *mut GcObjectHeader;
                // SAFETY: f is a valid GC reference; header_ptr points to a
                // registered GcObjectHeader in the GC's intrusive list.
                unsafe {
                    collector.mark_object(header_ptr);
                }
            }
            Value::Userdata(u) => {
                let header_ptr = u.as_ptr() as *mut GcObjectHeader;
                // SAFETY: u is a valid GC reference; header_ptr points to a
                // registered GcObjectHeader in the GC's intrusive list.
                unsafe {
                    collector.mark_object(header_ptr);
                }
            }
            Value::Thread(t) => {
                let header_ptr = t.as_ptr() as *mut GcObjectHeader;
                // SAFETY: t is a valid GC reference; header_ptr points to a
                // registered GcObjectHeader in the GC's intrusive list.
                unsafe {
                    collector.mark_object(header_ptr);
                }
            }
            // Nil, Boolean, Number, LightUserdata 不引用 GC 对象
            _ => {}
        }
    }
}

// =====================================================================
// Debug / Display
// =====================================================================

impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("array_size", &self.array.len())
            .field("hash_size", &self.hash.len())
            .field("metatable", &self.metatable)
            .field("flags", &self.flags)
            .finish()
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
    use crate::gc_string::GcString;

    // ── 辅助函数 ────────────────────────────────────────────────

    /// 创建一个测试用的 Table 并注册到 GC
    fn create_test_table(gc: &mut GarbageCollector) -> GcRef<Table> {
        gc.create(Table::new())
    }

    /// 创建一个测试用的字符串并注册到 GC
    fn create_test_string(gc: &mut GarbageCollector, s: &str) -> GcRef<GcString> {
        gc.create(GcString::new(s))
    }

    // ── 创建和基本属性测试 ─────────────────────────────────────

    #[test]
    fn test_create_empty_table() {
        let mut gc = GarbageCollector::new();
        let table_ref = create_test_table(&mut gc);

        // SAFETY: table is valid, GC won't run during this test
        let table = unsafe { &*table_ref.as_ptr() };
        assert_eq!(table.array_size(), 0);
        assert_eq!(table.hash_size(), 0);
        assert_eq!(table.total_size(), 0);
        assert!(table.metatable().is_none());
        assert_eq!(table.flags(), 0);
    }

    #[test]
    fn test_table_gc_header_type() {
        let table = Table::new();
        assert_eq!(table.gc_header().gc_type(), GcObjectType::Table);
    }

    #[test]
    fn test_table_with_capacity() {
        let table = Table::with_capacity(10, 20);
        assert_eq!(table.array_size(), 0);
        assert_eq!(table.hash_size(), 0);
        assert!(table.array.capacity() >= 10);
        assert!(table.hash.capacity() >= 20);
    }

    // ── 数组操作测试 ────────────────────────────────────────────

    #[test]
    fn test_array_set_get() {
        let mut table = Table::new();

        table.set_array(1, &Value::Number(10.0));
        table.set_array(2, &Value::Number(20.0));
        table.set_array(3, &Value::Number(30.0));

        assert_eq!(table.get_array(1), Value::Number(10.0));
        assert_eq!(table.get_array(2), Value::Number(20.0));
        assert_eq!(table.get_array(3), Value::Number(30.0));
        assert_eq!(table.array_size(), 3);
    }

    #[test]
    fn test_array_out_of_bounds_returns_nil() {
        let table = Table::new();
        assert_eq!(table.get_array(1), Value::Nil);
        assert_eq!(table.get_array(100), Value::Nil);
    }

    #[test]
    fn test_array_zero_or_negative_returns_nil() {
        let table = Table::new();
        assert_eq!(table.get_array(0), Value::Nil);
        assert_eq!(table.get_array(-1), Value::Nil);
    }

    #[test]
    fn test_set_array_zero_or_negative_ignored() {
        let mut table = Table::new();
        table.set_array(0, &Value::Number(42.0));
        table.set_array(-1, &Value::Number(42.0));
        assert_eq!(table.array_size(), 0);
    }

    #[test]
    fn test_array_auto_expand() {
        let mut table = Table::new();

        // 直接设置索引 5，应该自动扩展
        table.set_array(5, &Value::Number(50.0));

        assert_eq!(table.array_size(), 5);
        assert_eq!(table.get_array(5), Value::Number(50.0));
        // 中间的空位应该是 nil
        assert_eq!(table.get_array(1), Value::Nil);
        assert_eq!(table.get_array(2), Value::Nil);
        assert_eq!(table.get_array(3), Value::Nil);
        assert_eq!(table.get_array(4), Value::Nil);
    }

    #[test]
    fn test_array_lua_one_based() {
        let mut table = Table::new();
        table.set_array(1, &Value::Number(100.0));
        // 索引 0 不是有效索引（Lua 数组从 1 开始）
        assert_eq!(table.get_array(0), Value::Nil);
        assert_eq!(table.get_array(1), Value::Number(100.0));
    }

    // ── 哈希操作测试 ────────────────────────────────────────────

    #[test]
    fn test_hash_set_get() {
        let mut gc = GarbageCollector::new();
        let mut table = Table::new();

        let key_str = create_test_string(&mut gc, "mykey");
        let key = Value::String(key_str);
        let val = Value::Boolean(true);

        table.set(&key, &val);

        let retrieved = table.get(&key);
        assert_eq!(retrieved, Value::Boolean(true));
    }

    #[test]
    fn test_hash_get_missing_key_returns_nil() {
        let table = Table::new();
        let key = Value::Number(99.0);
        assert_eq!(table.get(&key), Value::Nil);
    }

    #[test]
    fn test_hash_string_key() {
        let mut gc = GarbageCollector::new();
        let mut table = Table::new();

        let s1 = create_test_string(&mut gc, "hello");
        let s2 = create_test_string(&mut gc, "world");

        table.set(&Value::String(s1), &Value::Number(1.0));
        table.set(&Value::String(s2), &Value::Number(2.0));

        assert_eq!(table.get(&Value::String(s1)), Value::Number(1.0));
        assert_eq!(table.get(&Value::String(s2)), Value::Number(2.0));
    }

    #[test]
    fn test_hash_string_key_matches_by_content_without_interning() {
        let mut gc = GarbageCollector::new();
        let mut table = Table::new();

        let s1 = create_test_string(&mut gc, "same");
        let s2 = create_test_string(&mut gc, "same");

        assert_ne!(s1, s2);

        table.set(&Value::String(s1), &Value::Number(1.0));
        table.set(&Value::String(s2), &Value::Number(2.0));

        assert_eq!(table.hash_size(), 1);
        assert_eq!(table.get(&Value::String(s1)), Value::Number(2.0));
        assert_eq!(table.get(&Value::String(s2)), Value::Number(2.0));
        assert!(table.has(&Value::String(s2)));

        table.remove(&Value::String(s2));
        assert_eq!(table.get(&Value::String(s1)), Value::Nil);
        assert_eq!(table.hash_size(), 0);
    }

    #[test]
    fn test_hash_number_key() {
        let mut table = Table::new();
        // 非整数或超出范围的数字键应进入哈希部分
        table.set(&Value::Number(3.14), &Value::Boolean(true));
        assert_eq!(table.get(&Value::Number(3.14)), Value::Boolean(true));
        assert_eq!(table.hash_size(), 1);
    }

    // ── 通用 get/set 测试 ───────────────────────────────────────

    #[test]
    fn test_set_routes_integer_keys_to_array() {
        let mut table = Table::new();

        // 小正整数 → 数组部分
        table.set(&Value::Number(1.0), &Value::Number(10.0));
        table.set(&Value::Number(2.0), &Value::Number(20.0));

        assert_eq!(table.array_size(), 2);
        assert_eq!(table.hash_size(), 0);
        assert_eq!(table.get(&Value::Number(1.0)), Value::Number(10.0));
        assert_eq!(table.get(&Value::Number(2.0)), Value::Number(20.0));
    }

    #[test]
    fn test_set_nil_value_removes_key() {
        let mut table = Table::new();

        table.set(&Value::Number(1.0), &Value::Number(42.0));
        assert_eq!(table.array_size(), 1);
        assert!(table.has(&Value::Number(1.0)));

        // nil value → remove
        table.set(&Value::Number(1.0), &Value::Nil);
        assert!(!table.has(&Value::Number(1.0)));
        assert_eq!(table.array_size(), 1); // 数组不会收缩
        assert_eq!(table.get_array(1), Value::Nil);
    }

    #[test]
    #[should_panic(expected = "table index is nil")]
    fn test_set_nil_key_panics() {
        let mut table = Table::new();
        table.set(&Value::Nil, &Value::Number(42.0));
    }

    #[test]
    #[should_panic(expected = "table index is NaN")]
    fn test_set_nan_key_panics() {
        let mut table = Table::new();
        table.set(&Value::Number(LuaNumber::NAN), &Value::Number(42.0));
    }

    #[test]
    fn test_set_overwrites_existing_key() {
        let mut table = Table::new();

        table.set(&Value::Number(1.0), &Value::Number(10.0));
        table.set(&Value::Number(1.0), &Value::Number(99.0));

        assert_eq!(table.get(&Value::Number(1.0)), Value::Number(99.0));
    }

    // ── has 测试 ────────────────────────────────────────────────

    #[test]
    fn test_has_existing_key() {
        let mut table = Table::new();
        table.set(&Value::Number(1.0), &Value::Boolean(true));
        assert!(table.has(&Value::Number(1.0)));
    }

    #[test]
    fn test_has_missing_key() {
        let table = Table::new();
        assert!(!table.has(&Value::Number(1.0)));
    }

    #[test]
    fn test_has_nil_value_key() {
        let mut table = Table::new();
        // 设置后再设为 nil
        table.set(&Value::Number(1.0), &Value::Number(42.0));
        table.set(&Value::Number(1.0), &Value::Nil);
        assert!(!table.has(&Value::Number(1.0)));
    }

    // ── remove 测试 ─────────────────────────────────────────────

    #[test]
    fn test_remove_array_key() {
        let mut table = Table::new();
        table.set(&Value::Number(1.0), &Value::Number(100.0));
        assert!(table.has(&Value::Number(1.0)));

        table.remove(&Value::Number(1.0));
        assert!(!table.has(&Value::Number(1.0)));
    }

    #[test]
    fn test_remove_hash_key() {
        let mut table = Table::new();
        let key = Value::Number(3.14);
        table.set(&key, &Value::Boolean(true));
        assert!(table.has(&key));

        table.remove(&key);
        assert!(!table.has(&key));
    }

    // ── clear 测试 ──────────────────────────────────────────────

    #[test]
    fn test_clear_table() {
        let mut table = Table::new();
        table.set(&Value::Number(1.0), &Value::Number(10.0));
        table.set(&Value::Number(2.0), &Value::Number(20.0));
        table.set(&Value::Number(3.14), &Value::Boolean(true));

        assert_eq!(table.total_size(), 3);

        table.clear();
        assert_eq!(table.array_size(), 0);
        assert_eq!(table.hash_size(), 0);
        assert_eq!(table.total_size(), 0);
        assert!(table.metatable().is_none());
    }

    // ── 元表测试 ────────────────────────────────────────────────

    #[test]
    fn test_metatable_set_get() {
        let mut gc = GarbageCollector::new();
        let mt_ref = create_test_table(&mut gc);

        let mut table = Table::new();
        assert!(table.metatable().is_none());

        table.set_metatable(Some(mt_ref));
        assert!(table.metatable().is_some());
        assert_eq!(table.metatable().unwrap(), mt_ref);
    }

    #[test]
    fn test_metatable_remove() {
        let mut gc = GarbageCollector::new();
        let mt_ref = create_test_table(&mut gc);

        let mut table = Table::new();
        table.set_metatable(Some(mt_ref));
        assert!(table.metatable().is_some());

        table.set_metatable(None);
        assert!(table.metatable().is_none());
    }

    // ── flags 测试 ──────────────────────────────────────────────

    #[test]
    fn test_flags_default() {
        let table = Table::new();
        assert_eq!(table.flags(), 0);
    }

    #[test]
    fn test_flags_set_get() {
        let mut table = Table::new();
        table.set_flags(0xFF);
        assert_eq!(table.flags(), 0xFF);

        table.set_flags(0x00);
        assert_eq!(table.flags(), 0x00);
    }

    #[test]
    fn test_set_resets_flags() {
        let mut table = Table::new();
        table.set_flags(0xFF);
        assert_eq!(table.flags(), 0xFF);

        // set 操作应该重置 flags
        table.set(&Value::Number(1.0), &Value::Number(42.0));
        assert_eq!(table.flags(), 0);
    }

    // ── length 测试 ─────────────────────────────────────────────

    #[test]
    fn test_length_empty_table() {
        let table = Table::new();
        assert_eq!(table.length(), 0);
    }

    #[test]
    fn test_length_dense_array() {
        let mut table = Table::new();
        for i in 1..=5 {
            table.set_array(i, &Value::Number(i as LuaNumber));
        }
        assert_eq!(table.length(), 5);
    }

    #[test]
    fn test_length_with_nil_holes() {
        let mut table = Table::new();
        table.set_array(1, &Value::Number(10.0));
        // 索引 2 是隐式 nil（数组自动扩展填充）
        table.set_array(3, &Value::Number(30.0));
        // C++ 实现: array = [10.0, nil, 30.0]; arraySize = 3
        // array[2] (最后一个) 非 nil, hasIntegerKey(4) = false
        // → 返回 3（对齐 C++ 行为）
        let len = table.length();
        assert_eq!(len, 3);
    }

    #[test]
    fn test_length_all_nil_array() {
        let mut table = Table::new();
        table.set_array(1, &Value::Nil);
        table.set_array(2, &Value::Nil);
        assert_eq!(table.length(), 0);
    }

    // ── next 迭代器测试 ─────────────────────────────────────────

    #[test]
    fn test_next_empty_table() {
        let table = Table::new();
        assert!(table.next(&Value::Nil).is_none());
    }

    #[test]
    fn test_next_array_only() {
        let mut table = Table::new();
        table.set_array(1, &Value::Number(10.0));
        table.set_array(2, &Value::Number(20.0));

        // 从 nil 开始 → 返回第一个元素
        let (k1, v1) = table.next(&Value::Nil).unwrap();
        assert_eq!(k1, Value::Number(1.0));
        assert_eq!(v1, Value::Number(10.0));

        // 从 k1 继续 → 返回第二个元素
        let (k2, v2) = table.next(&k1).unwrap();
        assert_eq!(k2, Value::Number(2.0));
        assert_eq!(v2, Value::Number(20.0));
    }

    #[test]
    fn test_next_skips_nil_array_elements() {
        let mut table = Table::new();
        table.set_array(1, &Value::Nil); // nil, should be skipped
        table.set_array(2, &Value::Number(20.0));

        let (k, v) = table.next(&Value::Nil).unwrap();
        assert_eq!(k, Value::Number(2.0));
        assert_eq!(v, Value::Number(20.0));
    }

    #[test]
    fn test_next_hash_only() {
        let mut table = Table::new();
        let key = Value::Number(3.14);
        table.set(&key, &Value::Boolean(true));

        let (k, v) = table.next(&Value::Nil).unwrap();
        assert_eq!(k, key);
        assert_eq!(v, Value::Boolean(true));

        // 从 k 继续 → 结束
        assert!(table.next(&k).is_none());
    }

    // ── GcObject trait 测试 ─────────────────────────────────────

    #[test]
    fn test_table_gc_object_trait() {
        let table = Table::new();
        assert_eq!(table.gc_header().gc_type(), GcObjectType::Table);
        assert!(table.gc_header().is_gray()); // 初始状态: 灰色
    }

    #[test]
    fn test_table_get_size() {
        let mut table = Table::new();
        // 空表
        let empty_size = table.get_size();
        assert!(empty_size >= std::mem::size_of::<Table>());

        // 添加数据后
        table.set_array(1, &Value::Number(42.0));
        table.set(&Value::Number(3.14), &Value::Boolean(true));
        let filled_size = table.get_size();
        assert!(filled_size > empty_size);
    }

    #[test]
    fn test_table_mark_children() {
        let mut gc = GarbageCollector::new();
        let s_ref = create_test_string(&mut gc, "test_key");
        let mt_ref = create_test_table(&mut gc);

        let mut table = Table::new();
        table.set(&Value::String(s_ref), &Value::Number(1.0));
        table.set_metatable(Some(mt_ref));

        // 注册 table 到 GC（mark_children 需要 GarbageCollector）
        let table_ref = gc.create(table);

        // 重置标记
        gc.reset_marks();

        // 标记 table 的子对象
        unsafe {
            let table = &*table_ref.as_ptr();
            table.mark_children(&mut gc);
        }

        // 验证：字符串和元表应该被标记为灰色（非白色）
        unsafe {
            let s_header = s_ref.as_ptr() as *mut GcObjectHeader;
            assert!(!(*s_header).is_white(), "String should be marked");

            let mt_header = mt_ref.as_ptr() as *mut GcObjectHeader;
            assert!(!(*mt_header).is_white(), "Metatable should be marked");
        }
    }

    #[test]
    fn test_table_default() {
        let table = Table::default();
        assert_eq!(table.array_size(), 0);
        assert_eq!(table.hash_size(), 0);
        assert!(table.metatable().is_none());
    }

    // ── is_array_index 测试 ─────────────────────────────────────

    #[test]
    fn test_is_array_index_valid_integer() {
        let key = Value::Number(5.0);
        let mut idx: i32 = 0;
        assert!(Table::is_array_index(&key, &mut idx));
        assert_eq!(idx, 5);
    }

    #[test]
    fn test_is_array_index_zero_rejected() {
        let key = Value::Number(0.0);
        let mut idx: i32 = 0;
        assert!(!Table::is_array_index(&key, &mut idx));
    }

    #[test]
    fn test_is_array_index_negative_rejected() {
        let key = Value::Number(-1.0);
        let mut idx: i32 = 0;
        assert!(!Table::is_array_index(&key, &mut idx));
    }

    #[test]
    fn test_is_array_index_non_integer_rejected() {
        let key = Value::Number(3.14);
        let mut idx: i32 = 0;
        assert!(!Table::is_array_index(&key, &mut idx));
    }

    #[test]
    fn test_is_array_index_non_number_rejected() {
        let key = Value::Boolean(true);
        let mut idx: i32 = 0;
        assert!(!Table::is_array_index(&key, &mut idx));
    }

    #[test]
    fn test_is_array_index_out_of_range_rejected() {
        let key = Value::Number(2_000_000.0);
        let mut idx: i32 = 0;
        assert!(!Table::is_array_index(&key, &mut idx));
    }

    // ── Value Hash 一致性测试 ──────────────────────────────────

    #[test]
    fn test_value_hash_same_values_same_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();

        let v1 = Value::Number(42.0);
        let v2 = Value::Number(42.0);

        v1.hash(&mut hasher1);
        v2.hash(&mut hasher2);

        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_value_hash_different_types_different_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // 不同类型的值应该有不同哈希（尽管不保证，但我们测试典型情况）
        let nil_hash = {
            let mut h = DefaultHasher::new();
            Value::Nil.hash(&mut h);
            h.finish()
        };
        let bool_hash = {
            let mut h = DefaultHasher::new();
            Value::Boolean(true).hash(&mut h);
            h.finish()
        };
        assert_ne!(nil_hash, bool_hash);
    }
}
