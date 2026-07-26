//! Immutable byte-string storage for Lua string values.
//!
//! Lua strings are arbitrary byte sequences. [`ByteString`] therefore does
//! not require UTF-8 and treats embedded NUL bytes as ordinary data. The
//! backing allocation contains one additional trailing NUL sentinel, while
//! every logical operation excludes that sentinel.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::Utf8Error;

/// An immutable Lua byte string with a stable, trailing-NUL-terminated buffer.
///
/// The trailing NUL is storage metadata, not part of the logical value. An
/// embedded NUL remains part of the value, so [`ByteString::as_ptr`] must be
/// paired with [`ByteString::len`] rather than treated as a C string.
#[derive(Clone)]
pub struct ByteString {
    /// Always contains at least the trailing NUL sentinel.
    storage: Box<[u8]>,
}

impl ByteString {
    /// Copies an arbitrary byte sequence into immutable storage.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut storage = Vec::with_capacity(bytes.len() + 1);
        storage.extend_from_slice(bytes);
        storage.push(0);
        Self {
            storage: storage.into_boxed_slice(),
        }
    }

    /// Copies UTF-8 text without changing its encoded bytes.
    #[must_use]
    pub fn from_utf8_text(text: &str) -> Self {
        Self::from_bytes(text.as_bytes())
    }

    /// Copies a static ASCII byte sequence.
    ///
    /// This constructor is intended for internal Lua keywords and other
    /// protocol constants. It panics when the supplied constant is not ASCII.
    #[must_use]
    pub fn from_static_ascii(bytes: &'static [u8]) -> Self {
        assert!(
            bytes.is_ascii(),
            "ByteString::from_static_ascii requires ASCII input"
        );
        Self::from_bytes(bytes)
    }

    /// Returns the logical bytes, excluding the trailing NUL sentinel.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.storage[..self.storage.len() - 1]
    }

    /// Returns the logical byte length.
    #[must_use]
    pub fn len(&self) -> usize {
        self.storage.len() - 1
    }

    /// Returns whether the logical byte sequence is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a stable pointer to the start of the backing buffer.
    ///
    /// The byte at `as_ptr().add(len())` is the trailing NUL sentinel for the
    /// lifetime of this value. Embedded NUL bytes are valid, so callers must
    /// retain the explicit length.
    #[must_use]
    pub fn as_ptr(&self) -> *const u8 {
        self.storage.as_ptr()
    }

    /// Borrows the logical bytes as UTF-8 when they form valid UTF-8.
    pub fn to_utf8(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(self.as_bytes())
    }

    /// Converts the logical bytes for human-facing display, replacing invalid
    /// UTF-8 sequences with the Unicode replacement character.
    #[must_use]
    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(self.as_bytes())
    }
}

impl PartialEq for ByteString {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for ByteString {}

impl PartialOrd for ByteString {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ByteString {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Hash for ByteString {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl AsRef<[u8]> for ByteString {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Borrow<[u8]> for ByteString {
    fn borrow(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl fmt::Debug for ByteString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ByteString")
            .field(&self.as_bytes())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;

    fn hash_of<T: Hash + ?Sized>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn empty_value_has_only_the_storage_sentinel() {
        let value = ByteString::from_bytes(&[]);

        assert!(value.is_empty());
        assert_eq!(value.len(), 0);
        assert_eq!(value.as_bytes(), b"");
        assert_eq!(value.storage.as_ref(), &[0]);
    }

    #[test]
    fn embedded_nul_is_logical_data() {
        let value = ByteString::from_bytes(b"a\0b\0");

        assert_eq!(value.len(), 4);
        assert_eq!(value.as_bytes(), b"a\0b\0");
        assert_eq!(value.storage.as_ref(), b"a\0b\0\0");
    }

    #[test]
    fn high_bytes_are_preserved() {
        let value = ByteString::from_bytes(&[0x80, 0xfe, 0xff]);

        assert_eq!(value.as_bytes(), &[0x80, 0xfe, 0xff]);
        assert!(value.to_utf8().is_err());
    }

    #[test]
    fn invalid_utf8_is_preserved_and_reported() {
        let raw = [0xf0, 0x28, 0x8c, 0x28];
        let value = ByteString::from_bytes(&raw);

        assert_eq!(value.as_bytes(), raw);
        assert!(value.to_utf8().is_err());
    }

    #[test]
    fn every_byte_value_round_trips() {
        let raw: Vec<u8> = (0..=u8::MAX).collect();
        let value = ByteString::from_bytes(&raw);

        assert_eq!(value.len(), 256);
        assert_eq!(value.as_bytes(), raw);
    }

    #[test]
    fn pointer_length_and_sentinel_share_one_stable_buffer() {
        let value = ByteString::from_bytes(&[0x00, 0x7f, 0x80, 0xff]);
        let pointer = value.as_ptr();

        assert_eq!(pointer, value.as_ptr());
        assert_eq!(pointer, value.as_bytes().as_ptr());

        // SAFETY: ByteString owns `len + 1` initialized bytes at `pointer`;
        // the final byte is its invariant trailing sentinel.
        let storage = unsafe { std::slice::from_raw_parts(pointer, value.len() + 1) };
        assert_eq!(storage, &[0x00, 0x7f, 0x80, 0xff, 0x00]);

        let _ = value.to_string_lossy();
        assert_eq!(pointer, value.as_ptr());
    }

    #[test]
    fn equality_hash_and_order_use_only_logical_bytes() {
        let first = ByteString::from_bytes(&[0x00, 0xff]);
        let same = ByteString::from_bytes(&[0x00, 0xff]);
        let greater = ByteString::from_bytes(&[0x01]);

        assert_eq!(first, same);
        assert_ne!(first, greater);
        assert_eq!(hash_of(&first), hash_of(first.as_bytes()));
        assert_eq!(hash_of(&first), hash_of(&same));
        assert!(first < greater);

        let as_ref: &[u8] = first.as_ref();
        let borrowed: &[u8] = first.borrow();
        assert_eq!(as_ref, first.as_bytes());
        assert_eq!(borrowed, first.as_bytes());
    }

    #[test]
    fn utf8_constructor_and_views_do_not_reencode() {
        let text = "Lua 字节";
        let value = ByteString::from_utf8_text(text);

        assert_eq!(value.as_bytes(), text.as_bytes());
        assert_eq!(value.to_utf8(), Ok(text));
        assert_eq!(value.to_string_lossy(), text);
    }

    #[test]
    fn static_ascii_constructor_accepts_protocol_constants() {
        let value = ByteString::from_static_ascii(b"__index");

        assert_eq!(value.as_bytes(), b"__index");
    }

    #[test]
    #[should_panic(expected = "requires ASCII input")]
    fn static_ascii_constructor_rejects_non_ascii_constants() {
        let _ = ByteString::from_static_ascii(b"\xff");
    }

    #[test]
    fn lossy_conversion_is_explicit_and_debug_is_byte_oriented() {
        let value = ByteString::from_bytes(&[b'a', 0, 0xff, b'b']);

        assert_eq!(value.to_string_lossy(), "a\0\u{fffd}b");
        assert_eq!(format!("{value:?}"), "ByteString([97, 0, 255, 98])");
    }
}
