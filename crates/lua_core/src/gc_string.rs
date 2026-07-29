//! GC 管理的字符串对象
//!
//! `GcString` 保存 Lua 的任意字节字符串并预计算哈希值。驻留身份由
//! `StringPool` 提供；对象本身只保留唯一的规范字节载荷。
//!

use crate::byte_string::ByteString;
use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::header::GcObjectHeader;
use crate::types::GcObjectType;
use std::borrow::Cow;
use std::str::Utf8Error;

/// Lua 5.1 哈希采样阈值
/// 长度超过此值的字符串采用采样方式计算哈希
const HASH_LIMIT: usize = 5;

/// GC 管理的字符串对象
///
/// 内存布局（`#[repr(C)]`，header 必须在开头）:
/// - header: GcObjectHeader (16 bytes)
/// - hash: usize (8 bytes)
/// - bytes: ByteString（任意字节、尾随 NUL 哨兵）
///
/// 通过 `StringPool` 创建时的驻留保证:
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

    /// Lua 字符串的规范载荷。所有身份相关操作都以这些字节为准。
    bytes: ByteString,
}

impl GcString {
    /// 从任意 Lua 字节序列创建字符串，不要求 UTF-8。
    ///
    /// This raw constructor is crate-internal so external production code
    /// cannot bypass canonical `StringPool` interning.
    #[must_use]
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_byte_string(ByteString::from_bytes(bytes))
    }

    fn from_byte_string(bytes: ByteString) -> Self {
        let hash = Self::compute_hash_bytes(bytes.as_bytes());
        Self {
            header: GcObjectHeader::new(GcObjectType::String),
            hash,
            bytes,
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
        self.bytes.len()
    }

    /// 检查字符串是否为空
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// 获取 Lua 字符串的完整逻辑字节。
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    /// 在载荷为合法 UTF-8 时返回文本视图。
    pub fn to_utf8(&self) -> Result<&str, Utf8Error> {
        self.bytes.to_utf8()
    }

    /// 为诊断和显示生成显式的有损 UTF-8 文本。
    pub fn to_string_lossy(&self) -> Cow<'_, str> {
        self.bytes.to_string_lossy()
    }

    /// 获取规范字节载荷的稳定指针。
    ///
    /// `as_ptr().add(len())` 是尾随 NUL 哨兵；内嵌 NUL 仍属于载荷。
    #[inline]
    pub fn as_ptr(&self) -> *const u8 {
        self.bytes.as_ptr()
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

    /// 计算固定 `lua_cpp` oracle 使用的字符串哈希。
    ///
    /// 该实现从索引 0 开始按 `(len >> 5) + 1` 前向采样。所有算术都
    /// 使用 `usize` wrapping 语义，以精确复刻 C++ 无符号整数运算。
    #[must_use]
    pub fn compute_hash_bytes(bytes: &[u8]) -> usize {
        let l = bytes.len();

        // 种子值 = 字符串长度
        let mut h: usize = l;

        // 采样步长: 对于短字符串 step=1（每个字节参与哈希）
        // 对于长字符串 step > 1（每隔 step 字节取一个）
        let step = (l >> HASH_LIMIT) + 1;

        for byte in bytes.iter().step_by(step) {
            // h = h ^ ((h << 5) + (h >> 2) + byte)
            h ^= (h << 5)
                .wrapping_add(h >> 2)
                .wrapping_add(usize::from(*byte));
        }

        h
    }
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
        std::mem::size_of::<Self>() + self.bytes.len() + 1
    }
}

// =====================================================================
// 标准 trait 实现
// =====================================================================

impl std::fmt::Debug for GcString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcString")
            .field("hash", &self.hash)
            .field("length", &self.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl std::fmt::Display for GcString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}

impl PartialEq for GcString {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for GcString {}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gc_string_from_utf8_text() {
        let s = GcString::from_bytes(b"hello");
        assert_eq!(s.len(), 5);
        assert_eq!(s.as_bytes(), b"hello");
        assert_eq!(s.to_utf8(), Ok("hello"));
        assert!(!s.is_empty());
    }

    #[test]
    fn test_gc_string_empty() {
        let s = GcString::from_bytes(b"");
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn test_hash_same_content_same_hash() {
        let s1 = GcString::from_bytes(b"hello world");
        let s2 = GcString::from_bytes(b"hello world");
        assert_eq!(s1.hash(), s2.hash());
    }

    #[test]
    fn test_hash_different_content_different_hash() {
        let s1 = GcString::from_bytes(b"hello");
        let s2 = GcString::from_bytes(b"world");
        assert_ne!(s1.hash(), s2.hash());
    }

    #[test]
    fn test_hash_zero_length() {
        let s = GcString::from_bytes(b"");
        // Hash of empty string is just the seed (length = 0)
        assert_eq!(s.hash(), 0);
    }

    #[test]
    fn test_hash_long_string() {
        // Long string that triggers sampling (> 32 bytes)
        let long = vec![b'a'; 100];
        let s = GcString::from_bytes(&long);
        assert_ne!(s.hash(), 0);
    }

    #[test]
    fn test_mark_fixed() {
        let s = GcString::from_bytes(b"fixed");
        assert!(!s.is_fixed());
        s.mark_fixed();
        assert!(s.is_fixed());
    }

    #[test]
    fn test_gc_header_type() {
        let s = GcString::from_bytes(b"test");
        assert_eq!(s.header.gc_type(), GcObjectType::String);
    }

    #[test]
    fn test_compute_hash_deterministic() {
        let h1 = GcString::compute_hash_bytes(b"lua");
        let h2 = GcString::compute_hash_bytes(b"lua");
        assert_eq!(h1, h2);
    }

    #[test]
    fn arbitrary_bytes_define_length_pointer_hash_and_equality() {
        let raw: Vec<u8> = (0..=u8::MAX).collect();
        let first = GcString::from_bytes(&raw);
        let same = GcString::from_bytes(&raw);
        let different = GcString::from_bytes(&raw[..raw.len() - 1]);

        assert_eq!(first.len(), raw.len());
        assert_eq!(first.as_bytes(), raw);
        assert_eq!(first.hash(), GcString::compute_hash_bytes(&raw));
        assert_eq!(first, same);
        assert_ne!(first, different);

        // SAFETY: ByteString guarantees `len + 1` initialized bytes and a
        // trailing NUL sentinel in the same stable allocation.
        let storage = unsafe { std::slice::from_raw_parts(first.as_ptr(), first.len() + 1) };
        assert_eq!(&storage[..first.len()], raw);
        assert_eq!(storage[first.len()], 0);
    }

    #[test]
    fn embedded_nul_and_invalid_utf8_are_preserved() {
        let raw = [b'a', 0, 0xff, 0x80, b'b'];
        let value = GcString::from_bytes(&raw);

        assert_eq!(value.as_bytes(), raw);
        assert!(value.to_utf8().is_err());
        assert_eq!(value.to_string_lossy(), "a\0��b");
    }

    #[test]
    fn utf8_constructor_preserves_encoded_bytes() {
        let text = "Lua 字节";
        let value = GcString::from_bytes(text.as_bytes());

        assert_eq!(value.as_bytes(), text.as_bytes());
        assert_eq!(value.to_utf8(), Ok(text));
    }

    #[test]
    fn size_accounts_for_one_canonical_payload_and_sentinel() {
        let raw = [0x00, 0x80, 0xff, b'x'];
        let value = GcString::from_bytes(&raw);

        assert_eq!(
            value.get_size(),
            std::mem::size_of::<GcString>() + raw.len() + 1
        );
    }

    #[test]
    fn display_is_an_explicit_lossy_presentation() {
        let value = GcString::from_bytes(&[b'a', 0xff, b'b']);

        assert_eq!(value.to_string(), "a\u{fffd}b");
    }

    #[test]
    fn fixed_cpp_forward_hash_vectors_match() {
        let vectors: &[(&[u8], usize)] = &[
            (b"", 0),
            (b"a", 128),
            (b"ab", 5_193),
            (b"lua", 218_517),
            (&[0x00, 0xff, 0x80], 109_393),
        ];

        for &(bytes, expected) in vectors {
            assert_eq!(
                GcString::compute_hash_bytes(bytes),
                expected,
                "hash mismatch for {bytes:?}"
            );
        }
    }
}
