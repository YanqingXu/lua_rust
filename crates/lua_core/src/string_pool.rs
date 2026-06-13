//! 字符串驻留池
//!
//! `StringPool` 实现 Lua 字符串驻留（string interning）机制，
//! 确保相同内容的字符串在内存中只存储一份。
//!
//! 所有字符串通过 `intern()` 方法创建或获取，保证指针相等性。
//!
//! C++ 参考: `lua_cpp/src/core/string_pool.hpp`, `.cpp`

use std::collections::HashMap;

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_ref::GcRef;
use crate::gc_string::GcString;

/// 字符串驻留池
///
/// 管理所有 GC 字符串的创建和查找，实现字符串驻留机制。
///
/// 字符串驻留流程:
/// 1. 调用 `intern(str)` 请求获取/创建字符串
/// 2. 计算哈希值并在池中查找
/// 3. 如果已存在 → 返回已有 `GcRef<GcString>`
/// 4. 如果不存在 → 通过 GC 创建新 `GcString`，加入池，返回
///
/// C++ 对应: `StringPool`
pub struct StringPool {
    /// 字符串哈希表: key = 字符串内容, value = GC 引用
    /// 使用 owned String 作为 key（C++ 同样使用 Str 避免悬空引用）
    pool: HashMap<String, GcRef<GcString>>,
}

impl StringPool {
    /// 创建空的字符串池
    pub fn new() -> Self {
        Self {
            pool: HashMap::new(),
        }
    }

    /// 创建预分配容量的字符串池
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            pool: HashMap::with_capacity(capacity),
        }
    }

    // ── 字符串驻留接口 ────────────────────────────────────────

    /// 驻留字符串 — 获取或创建字符串对象
    ///
    /// 如果字符串已存在，返回已有的 `GcRef<GcString>`；
    /// 如果不存在，创建新的 `GcString` 并通过 GC 注册。
    ///
    /// C++ 对应: `StringPool::intern(StrView str)`
    pub fn intern(&mut self, gc: &mut GarbageCollector, s: &str) -> GcRef<GcString> {
        // 在池中查找是否已存在
        if let Some(&existing) = self.pool.get(s) {
            return existing;
        }

        // 不存在 → 创建新对象并通过 GC 注册
        let gc_string = GcString::new(s);
        let gc_ref: GcRef<GcString> = gc.create(gc_string);

        // 加入池中：使用 GcString 的 data 作为 key
        // SAFETY: gc_ref 刚创建，对象有效
        let data: &str = unsafe { &*gc_ref.as_ptr() }.data();
        self.pool.insert(data.to_string(), gc_ref);

        gc_ref
    }

    /// 查找字符串 — 不创建新对象
    ///
    /// C++ 对应: `StringPool::find(StrView str)`
    pub fn find(&self, s: &str) -> Option<GcRef<GcString>> {
        self.pool.get(s).copied()
    }

    /// 从池中移除字符串
    ///
    /// 当 GC 回收字符串时调用，从池中移除对应条目。
    ///
    /// C++ 对应: `StringPool::remove(GCString* str)`
    ///
    /// # Safety
    /// `gc_ref` 必须指向一个当前在池中的有效字符串。
    pub fn remove(&mut self, gc_ref: GcRef<GcString>) {
        if gc_ref.is_null() {
            return;
        }

        // SAFETY: caller guarantees gc_ref is valid and in pool
        let data: &str = unsafe { &*gc_ref.as_ptr() }.data();
        self.pool.remove(data);
    }

    // ── 容量管理 ──────────────────────────────────────────────

    /// 获取池中字符串数量
    pub fn len(&self) -> usize {
        self.pool.len()
    }

    /// 检查池是否为空
    pub fn is_empty(&self) -> bool {
        self.pool.is_empty()
    }

    /// 清空字符串池（不释放 GC 对象，由 GC 负责）
    pub fn clear(&mut self) {
        self.pool.clear();
    }

    /// 预分配哈希表空间
    ///
    /// C++ 对应: `StringPool::resize(usize newSize)`
    pub fn reserve(&mut self, additional: usize) {
        self.pool.reserve(additional);
    }

    // ── 迭代 ──────────────────────────────────────────────────

    /// 遍历所有已驻留的字符串
    pub fn for_each<F: FnMut(&str, GcRef<GcString>)>(&self, mut f: F) {
        for (key, &value) in &self.pool {
            f(key, value);
        }
    }
}

impl Default for StringPool {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StringPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StringPool")
            .field("size", &self.pool.len())
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

    #[test]
    fn test_intern_same_string_same_ptr() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s1 = pool.intern(&mut gc, "hello");
        let s2 = pool.intern(&mut gc, "hello");

        // 相同内容的字符串应该返回相同的 GcRef
        assert_eq!(s1, s2);
        assert_eq!(pool.len(), 1); // 只驻留了一份
    }

    #[test]
    fn test_intern_different_strings() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s1 = pool.intern(&mut gc, "hello");
        let s2 = pool.intern(&mut gc, "world");

        assert_ne!(s1, s2);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_intern_empty_string() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern(&mut gc, "");
        assert!(!s.is_null());
        // Safety: s is valid and in pool
        assert_eq!(unsafe { &*s.as_ptr() }.len(), 0);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_find_existing() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s1 = pool.intern(&mut gc, "find_me");
        let found = pool.find("find_me");

        assert!(found.is_some());
        assert_eq!(found.unwrap(), s1);
    }

    #[test]
    fn test_find_missing() {
        let pool = StringPool::new();

        let found = pool.find("not_there");
        assert!(found.is_none());
    }

    #[test]
    fn test_remove_string() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern(&mut gc, "removable");
        assert_eq!(pool.len(), 1);

        pool.remove(s);
        assert_eq!(pool.len(), 0);
        assert!(pool.find("removable").is_none());
    }

    #[test]
    fn test_remove_null_is_noop() {
        let mut pool = StringPool::new();
        pool.remove(GcRef::null());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn test_clear_pool() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        pool.intern(&mut gc, "a");
        pool.intern(&mut gc, "b");
        pool.intern(&mut gc, "c");
        assert_eq!(pool.len(), 3);

        pool.clear();
        assert_eq!(pool.len(), 0);
        assert!(pool.is_empty());
    }

    #[test]
    fn test_reserve_capacity() {
        let mut pool = StringPool::with_capacity(100);
        assert_eq!(pool.len(), 0);

        let mut gc = GarbageCollector::new();
        for i in 0..50 {
            pool.intern(&mut gc, &format!("str_{}", i));
        }
        assert_eq!(pool.len(), 50);
    }

    #[test]
    fn test_string_data_accessible() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern(&mut gc, "test_data");
        // SAFETY: s is valid
        let data = unsafe { &*s.as_ptr() }.data();
        assert_eq!(data, "test_data");
    }

    #[test]
    fn test_intern_preserves_hash() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern(&mut gc, "hash_test");
        // SAFETY: s is valid
        let hash = unsafe { &*s.as_ptr() }.hash();
        let expected = GcString::compute_hash("hash_test");
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_intern_string_with_null_bytes() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern(&mut gc, "a\0b");
        // SAFETY: s is valid
        let data = unsafe { &*s.as_ptr() }.data();
        assert_eq!(data.len(), 3);
        assert_eq!(data.as_bytes(), &[b'a', 0, b'b']);
    }
}
