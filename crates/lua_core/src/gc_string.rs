//! GC 管理的字符串对象
//!
//! `GcString` 实现 Lua 字符串驻留机制。相同内容的字符串在内存中
//! 只存储一份，创建时预计算哈希值，字符串比较可通过指针比较完成。
//!

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::header::GcObjectHeader;
use crate::types::GcObjectType;

/// Lua 5.1 哈希采样阈值
/// 长度超过此值的字符串采用采样方式计算哈希
const HASH_LIMIT: usize = 5;

/// GC 管理的字符串对象
///
/// 内存布局（`#[repr(C)]`，header 必须在开头）:
/// - header: GcObjectHeader (16 bytes)
/// - hash: usize (8 bytes)
/// - length: usize (8 bytes)
/// - data: String (~24 bytes heap-allocated)
///   总计约 56+ bytes
///
/// 字符串驻留保证:
/// - 相同内容的字符串返回相同指针
/// - 字符串不可变（无公开修改接口）
/// - 哈希值在创建时预计算
///
#[repr(C)]
pub struct GcString {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    /// 预计算的哈希值
    hash: usize,

    /// 字符串长度（字节数）
    length: usize,

    /// 字符串数据（owned，不可变）
    data: String,
}

impl GcString {
    /// 创建新的 GC 字符串
    ///
    /// 注意: 此构造器只应被 `StringPool` 调用。
    /// 直接使用会绕过驻留机制。
    pub fn new(s: &str) -> Self {
        let hash = Self::compute_hash(s);
        Self {
            header: GcObjectHeader::new(GcObjectType::String),
            hash,
            length: lua_byte_len(s),
            data: s.to_string(),
        }
    }

    // ── 访问器 ────────────────────────────────────────────────

    /// 获取预计算的哈希值
    #[inline]
    pub fn hash(&self) -> usize {
        self.hash
    }

    /// 获取字符串长度（字节数）
    #[inline]
    pub fn len(&self) -> usize {
        self.length
    }

    /// 检查字符串是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// 获取字符串数据
    #[inline]
    pub fn data(&self) -> &str {
        &self.data
    }

    /// 获取 C 风格字符串指针
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    /// 标记为固定字符串（防止 GC 回收）
    ///
    /// 固定字符串永远不会被 GC 回收，用于：
    /// - Lua 关键字（if, then, else 等）
    /// - 元方法名称（__index, __add 等）
    /// - 系统常量字符串
    #[inline]
    pub fn mark_fixed(&self) {
        self.header.mark_fixed();
    }

    /// 检查是否为固定字符串
    #[inline]
    pub fn is_fixed(&self) -> bool {
        self.header.is_fixed()
    }

    // ── 哈希计算 ──────────────────────────────────────────────

    /// 计算字符串的 Lua 5.1 哈希值
    ///
    /// 使用与 Lua 5.1 `luaS_hash` 完全相同的算法。
    /// 对于长字符串（> 32 字节），采用采样策略。
    ///
    pub fn compute_hash(s: &str) -> usize {
        let bytes = s.as_bytes();
        let l = bytes.len();

        // 种子值 = 字符串长度
        let mut h: usize = l;

        // 采样步长: 对于短字符串 step=1（每个字节参与哈希）
        // 对于长字符串 step > 1（每隔 step 字节取一个）
        let step = (l >> HASH_LIMIT) + 1;

        let mut remaining = l;
        while remaining >= step {
            remaining -= step;
            let byte = bytes[remaining] as usize;
            // h = h ^ ((h << 5) + (h >> 2) + byte)
            h ^= (h << 5).wrapping_add(h >> 2).wrapping_add(byte);
        }

        h
    }
}

fn lua_byte_len(s: &str) -> usize {
    s.chars()
        .map(|ch| {
            if (ch as u32) <= 0xff {
                1
            } else {
                ch.len_utf8()
            }
        })
        .sum()
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: GcString 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::String。
// 字符串不引用其他 GC 对象，因此 mark_children 为空。
unsafe impl GcObject for GcString {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    unsafe fn mark_children(&self, _collector: &mut GarbageCollector) {
        // 字符串对象不引用其他 GC 对象
    }

    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.data.capacity()
    }
}

// =====================================================================
// 标准 trait 实现
// =====================================================================

impl std::fmt::Debug for GcString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcString")
            .field("hash", &self.hash)
            .field("length", &self.length)
            .field("data", &self.data)
            .finish()
    }
}

impl std::fmt::Display for GcString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.data)
    }
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_string_new() {
        let s = GcString::new("hello");
        assert_eq!(s.len(), 5);
        assert_eq!(s.data(), "hello");
        assert!(!s.is_empty());
    }

    #[test]
    fn test_gc_string_empty() {
        let s = GcString::new("");
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.data(), "");
    }

    #[test]
    fn test_hash_same_content_same_hash() {
        let s1 = GcString::new("hello world");
        let s2 = GcString::new("hello world");
        assert_eq!(s1.hash(), s2.hash());
    }

    #[test]
    fn test_hash_different_content_different_hash() {
        let s1 = GcString::new("hello");
        let s2 = GcString::new("world");
        assert_ne!(s1.hash(), s2.hash());
    }

    #[test]
    fn test_hash_zero_length() {
        let s = GcString::new("");
        // Hash of empty string is just the seed (length = 0)
        assert_eq!(s.hash(), 0);
    }

    #[test]
    fn test_hash_long_string() {
        // Long string that triggers sampling (> 32 bytes)
        let long = "a".repeat(100);
        let s = GcString::new(&long);
        assert!(s.hash() != 0);
    }

    #[test]
    fn test_mark_fixed() {
        let s = GcString::new("fixed");
        assert!(!s.is_fixed());
        s.mark_fixed();
        assert!(s.is_fixed());
    }

    #[test]
    fn test_gc_header_type() {
        let s = GcString::new("test");
        assert_eq!(s.header.gc_type(), GcObjectType::String);
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let h1 = GcString::compute_hash("lua");
        let h2 = GcString::compute_hash("lua");
        assert_eq!(h1, h2);
    }
}
