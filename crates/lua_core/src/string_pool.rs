//! 字符串驻留池
//!
//! `StringPool` 实现 Lua 字符串驻留（string interning）机制，
//! 确保相同内容的字符串在内存中只存储一份。
//!
//! 所有字符串通过 `intern_bytes()` 方法创建或获取，保证指针相等性。
//!

use std::collections::HashMap;

use crate::byte_string::ByteString;
use crate::gc::collector::GarbageCollector;
use crate::gc::gc_ref::GcRef;
use crate::gc_string::GcString;

/// 字符串驻留池
///
/// 管理所有 GC 字符串的创建和查找，实现字符串驻留机制。
///
/// 字符串驻留流程:
/// 1. 调用 `intern_bytes(bytes)` 请求获取/创建字符串
/// 2. 计算哈希值并在池中查找
/// 3. 如果已存在 → 返回已有 `GcRef<GcString>`
/// 4. 如果不存在 → 通过 GC 创建新 `GcString`，加入池，返回
///
pub struct StringPool {
    /// 字符串哈希表: key = 字符串内容, value = GC 引用
    /// 使用 owned ByteString 作为 key，支持任意字节并避免悬空引用。
    pool: HashMap<ByteString, GcRef<GcString>>,
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

    /// 驻留任意 Lua 字节序列。
    ///
    /// 该接口不执行 UTF-8 验证或编码转换；相同字节内容总是返回同一
    /// `GcRef<GcString>`。
    pub fn intern_bytes(&mut self, gc: &mut GarbageCollector, bytes: &[u8]) -> GcRef<GcString> {
        if let Some(&existing) = self.pool.get(bytes) {
            if gc.contains_registered(existing) {
                return existing;
            }
            // A StringPool/collector mismatch or a previously detached pool
            // can leave a copied handle here. Evict it by owned byte key
            // without reading candidate object memory.
            self.pool.remove(bytes);
        }

        let gc_ref = gc.create(GcString::from_bytes(bytes));
        self.pool.insert(ByteString::from_bytes(bytes), gc_ref);
        gc_ref
    }

    /// 按完整字节内容查找字符串，不创建新对象。
    pub fn find_bytes(&self, bytes: &[u8]) -> Option<GcRef<GcString>> {
        self.pool.get(bytes).copied()
    }

    /// 从池中移除字符串
    ///
    /// 当 GC 回收字符串时调用，从池中移除对应条目。
    ///
    /// Removal compares allocation identity only and never dereferences the
    /// candidate, so stale and foreign handles are safe no-ops unless that
    /// exact identity is the canonical entry.
    pub fn remove(&mut self, gc_ref: GcRef<GcString>) {
        if gc_ref.is_null() {
            return;
        }

        self.pool.retain(|_, canonical| *canonical != gc_ref);
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
    pub fn reserve(&mut self, additional: usize) {
        self.pool.reserve(additional);
    }

    pub(crate) fn insert_reserved_bytes(&mut self, bytes: &[u8], value: GcRef<GcString>) {
        match self.pool.entry(ByteString::from_bytes(bytes)) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(value);
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                panic!("reserved StringPool publication unexpectedly found an entry");
            }
        }
    }

    // ── 迭代 ──────────────────────────────────────────────────

    /// 按规范字节视图遍历所有已驻留的字符串。
    pub fn for_each_bytes<F: FnMut(&[u8], GcRef<GcString>)>(&self, mut f: F) {
        for (key, &value) in &self.pool {
            f(key.as_bytes(), value);
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

        let s1 = pool.intern_bytes(&mut gc, b"hello");
        let s2 = pool.intern_bytes(&mut gc, b"hello");

        // 相同内容的字符串应该返回相同的 GcRef
        assert_eq!(s1, s2);
        assert_eq!(pool.len(), 1); // 只驻留了一份
    }

    #[test]
    fn test_intern_same_bytes_same_ptr() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let raw = [0x00, 0xff, 0x80, b'x'];

        let first = pool.intern_bytes(&mut gc, &raw);
        let second = pool.intern_bytes(&mut gc, &raw);

        assert_eq!(first, second);
        assert_eq!(first.as_ptr(), second.as_ptr());
        assert_eq!(pool.find_bytes(&raw), Some(first));
        assert_eq!(pool.len(), 1);

        // SAFETY: the interned object and its `len + 1` ByteString allocation
        // remain owned by `gc` for the duration of this test.
        let value = unsafe { &*first.as_ptr() };
        // SAFETY: ByteString guarantees the logical payload followed by one
        // initialized NUL sentinel in the same stable allocation.
        let storage = unsafe { std::slice::from_raw_parts(value.as_ptr(), value.len() + 1) };
        assert_eq!(&storage[..value.len()], raw);
        assert_eq!(storage[value.len()], 0);
    }

    #[test]
    fn test_intern_different_strings() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s1 = pool.intern_bytes(&mut gc, b"hello");
        let s2 = pool.intern_bytes(&mut gc, b"world");

        assert_ne!(s1, s2);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_intern_empty_string() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern_bytes(&mut gc, b"");
        assert!(!s.is_null());
        // Safety: s is valid and in pool
        assert_eq!(unsafe { &*s.as_ptr() }.len(), 0);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_find_existing() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s1 = pool.intern_bytes(&mut gc, b"find_me");
        let found = pool.find_bytes(b"find_me");

        assert!(found.is_some());
        assert_eq!(found.unwrap(), s1);
    }

    #[test]
    fn test_find_missing() {
        let pool = StringPool::new();

        let found = pool.find_bytes(b"not_there");
        assert!(found.is_none());
    }

    #[test]
    fn test_remove_string() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern_bytes(&mut gc, b"removable");
        assert_eq!(pool.len(), 1);

        pool.remove(s);
        assert_eq!(pool.len(), 0);
        assert!(pool.find_bytes(b"removable").is_none());
    }

    #[test]
    fn test_remove_noncanonical_duplicate_preserves_interned_identity() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let canonical = pool.intern_bytes(&mut gc, b"same-bytes");
        let duplicate = gc.create(GcString::from_bytes(b"same-bytes"));
        assert_ne!(canonical, duplicate);

        pool.remove(duplicate);

        assert_eq!(pool.find_bytes(b"same-bytes"), Some(canonical));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn test_remove_null_is_noop() {
        let mut pool = StringPool::new();
        pool.remove(GcRef::null());
        assert_eq!(pool.len(), 0);
    }

    #[test]
    fn intern_evicts_foreign_and_stale_pool_entries_before_reuse() {
        let mut first_gc = GarbageCollector::new();
        let mut second_gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let first = pool.intern_bytes(&mut first_gc, b"identity");
        let second = pool.intern_bytes(&mut second_gc, b"identity");
        assert_ne!(first, second);
        assert!(!second_gc.contains_registered(first));
        assert!(second_gc.contains_registered(second));
        assert_eq!(pool.find_bytes(b"identity"), Some(second));

        // Destroy through a deliberately different pool to inject a stale
        // copied entry into `pool` without dereferencing it.
        let mut destroy_pool = StringPool::new();
        assert_eq!(second_gc.sweep(&mut destroy_pool), 1);
        assert_eq!(pool.find_bytes(b"identity"), Some(second));

        let replacement = pool.intern_bytes(&mut second_gc, b"identity");
        assert_ne!(second, replacement);
        assert!(second_gc.contains_registered(replacement));
        assert_eq!(pool.find_bytes(b"identity"), Some(replacement));
    }

    #[test]
    fn remove_accepts_a_stale_handle_without_dereference() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let stale = pool.intern_bytes(&mut gc, b"stale-remove");
        let mut destroy_pool = StringPool::new();
        assert_eq!(gc.sweep(&mut destroy_pool), 1);

        pool.remove(stale);

        assert!(pool.is_empty());
    }

    #[test]
    fn test_clear_pool() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        pool.intern_bytes(&mut gc, b"a");
        pool.intern_bytes(&mut gc, b"b");
        pool.intern_bytes(&mut gc, b"c");
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
            let value = format!("str_{i}");
            pool.intern_bytes(&mut gc, value.as_bytes());
        }
        assert_eq!(pool.len(), 50);
    }

    #[test]
    fn test_string_data_accessible() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern_bytes(&mut gc, b"test_data");
        // SAFETY: s is valid
        let data = unsafe { &*s.as_ptr() }.as_bytes();
        assert_eq!(data, b"test_data");
    }

    #[test]
    fn test_intern_preserves_hash() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern_bytes(&mut gc, b"hash_test");
        // SAFETY: s is valid
        let hash = unsafe { &*s.as_ptr() }.hash();
        let expected = GcString::compute_hash_bytes(b"hash_test");
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_intern_all_byte_values_without_utf8_dependency() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let raw: Vec<u8> = (0..=u8::MAX).collect();

        let string = pool.intern_bytes(&mut gc, &raw);
        // SAFETY: string is live and retained by both gc and pool.
        let value = unsafe { &*string.as_ptr() };

        assert_eq!(value.as_bytes(), raw);
        assert_eq!(value.len(), 256);
        assert!(value.to_utf8().is_err());
        assert_eq!(pool.find_bytes(&raw), Some(string));
    }

    #[test]
    fn test_utf8_text_bytes_and_single_high_byte_are_distinct() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let utf8 = pool.intern_bytes(&mut gc, "é".as_bytes());
        let high_byte = pool.intern_bytes(&mut gc, &[0xe9]);
        let utf8_again = pool.intern_bytes(&mut gc, "é".as_bytes());

        assert_ne!(high_byte, utf8);
        assert_eq!(utf8_again, utf8);
        assert_eq!(pool.find_bytes(&[0xe9]), Some(high_byte));
        assert_eq!(pool.find_bytes("é".as_bytes()), Some(utf8));
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn test_intern_string_with_null_bytes() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let s = pool.intern_bytes(&mut gc, b"a\0b");
        // SAFETY: s is valid
        let data = unsafe { &*s.as_ptr() }.as_bytes();
        assert_eq!(data.len(), 3);
        assert_eq!(data, &[b'a', 0, b'b']);
        assert_eq!(pool.find_bytes(b"a\0b"), Some(s));
    }
}
