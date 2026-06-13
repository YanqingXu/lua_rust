//! GC 对象头部 — 侵入式链表节点
//!
//! `GcObjectHeader` 是每个 GC 管理对象的前缀，提供侵入式链表链接
//! 和三色标记算法的标记位。直接映射 C++ `GCObject` 的数据成员。
//!
//! C++ 参考: `lua_cpp/src/core/gc_object.hpp`

use std::cell::Cell;

use crate::types::GcObjectType;

// =====================================================================
// GC 标记位常量
// =====================================================================

/// GC 标记位定义 — 与 C++ `GCBits` 命名空间完全对齐
///
/// C++ 参考: `lua_cpp/src/core/gc_object.hpp` GCBits
pub mod bits {
    use crate::types::LuByte;

    pub const WHITE0BIT: u8 = 0;
    pub const WHITE1BIT: u8 = 1;
    pub const BLACKBIT: u8 = 2;
    pub const FINALIZEDBIT: u8 = 3;
    pub const WEAKKEYBIT: u8 = 4;
    pub const FIXEDBIT: u8 = 5;
    pub const WEAKVALUEBIT: u8 = 6;

    pub const WHITE0: LuByte = 1 << WHITE0BIT; // 0x01
    pub const WHITE1: LuByte = 1 << WHITE1BIT; // 0x02
    pub const BLACK: LuByte = 1 << BLACKBIT; // 0x04
    pub const WHITEBITS: LuByte = WHITE0 | WHITE1; // 0x03
    pub const FINALIZED: LuByte = 1 << FINALIZEDBIT; // 0x08
    pub const WEAKKEY: LuByte = 1 << WEAKKEYBIT; // 0x10
    pub const FIXED: LuByte = 1 << FIXEDBIT; // 0x20
    pub const WEAKVALUE: LuByte = 1 << WEAKVALUEBIT; // 0x40
    pub const WEAKBITS: LuByte = WEAKKEY | WEAKVALUE; // 0x50
}

// =====================================================================
// GcObjectHeader
// =====================================================================

/// GC 对象头部 — 侵入式链表节点 + GC 标记
///
/// 内存布局（`#[repr(C)]` 确保与 C++ 兼容）:
/// - `next`: 8 字节（64 位指针）
/// - `marked`: 1 字节（GC 标记位）
/// - `gc_type`: 1 字节（GcObjectType 枚举）
/// - padding: 6 字节（对齐到 8 字节边界）
///   总计: 16 字节
///
/// 使用 `Cell` 实现内部可变性，因为 GC 标记在共享引用下也需要修改。
///
/// C++ 对应: `GCObject` 的数据成员部分
#[repr(C)]
pub struct GcObjectHeader {
    /// 侵入式链表指针 — 指向下一个 GC 对象
    /// NULL 表示链表末尾
    next: Cell<*mut GcObjectHeader>,

    /// GC 标记位 — 三色标记 + 特殊标志位
    /// bit 0: White0, bit 1: White1, bit 2: Black,
    /// bit 3: Finalized, bit 4: WeakKey, bit 5: Fixed, bit 6: WeakValue
    marked: Cell<u8>,

    /// 对象类型标签
    gc_type: GcObjectType,
}

impl GcObjectHeader {
    /// 创建新的 GC 对象头部
    #[inline]
    pub fn new(gc_type: GcObjectType) -> Self {
        Self {
            next: Cell::new(std::ptr::null_mut()),
            marked: Cell::new(0),
            gc_type,
        }
    }

    // ── 链表管理 ──────────────────────────────────────────────

    /// 获取链表中的下一个对象指针
    #[inline]
    pub fn next(&self) -> *mut GcObjectHeader {
        self.next.get()
    }

    /// 设置链表中的下一个对象指针
    #[inline]
    pub fn set_next(&self, next: *mut GcObjectHeader) {
        self.next.set(next);
    }

    // ── 类型查询 ──────────────────────────────────────────────

    /// 获取 GC 对象类型
    #[inline]
    pub fn gc_type(&self) -> GcObjectType {
        self.gc_type
    }

    // ── GC 颜色管理 ──────────────────────────────────────────

    /// 获取原始标记位
    #[inline]
    pub fn marked(&self) -> u8 {
        self.marked.get()
    }

    /// 设置原始标记位
    #[inline]
    pub fn set_marked(&self, mark: u8) {
        self.marked.set(mark);
    }

    /// 获取 GC 颜色（从标记位推导）
    pub fn color(&self) -> crate::types::GcColor {
        let m = self.marked.get();
        if m & bits::BLACK != 0 {
            crate::types::GcColor::Black
        } else if m & bits::WHITEBITS != 0 {
            crate::types::GcColor::White
        } else {
            crate::types::GcColor::Gray
        }
    }

    /// 设置 GC 颜色
    ///
    /// C++ 对应: `GCObject::setColor(GCColor color)`
    pub fn set_color(&self, color: crate::types::GcColor) {
        let m = self.marked.get();
        // 清除颜色位
        let cleared = m & !(bits::WHITEBITS | bits::BLACK);
        // 设置新颜色
        let new_mark = match color {
            crate::types::GcColor::White => cleared | bits::WHITE0,
            crate::types::GcColor::Gray => cleared, // 灰色不设任何位
            crate::types::GcColor::Black => cleared | bits::BLACK,
        };
        self.marked.set(new_mark);
    }

    /// 检查是否为白色
    #[inline]
    pub fn is_white(&self) -> bool {
        (self.marked.get() & bits::WHITEBITS) != 0
    }

    /// 检查是否为灰色
    #[inline]
    pub fn is_gray(&self) -> bool {
        !self.is_white() && !self.is_black()
    }

    /// 检查是否为黑色
    #[inline]
    pub fn is_black(&self) -> bool {
        (self.marked.get() & bits::BLACK) != 0
    }

    /// 检查是否已标记（非白色）
    #[inline]
    pub fn is_marked(&self) -> bool {
        !self.is_white()
    }

    // ── 特殊标志位 ────────────────────────────────────────────

    /// 标记为固定对象（防止 GC 回收）
    #[inline]
    pub fn mark_fixed(&self) {
        self.marked.set(self.marked.get() | bits::FIXED);
    }

    /// 检查是否为固定对象
    #[inline]
    pub fn is_fixed(&self) -> bool {
        (self.marked.get() & bits::FIXED) != 0
    }

    /// 标记为终结
    #[inline]
    pub fn mark_finalized(&self) {
        self.marked.set(self.marked.get() | bits::FINALIZED);
    }

    /// 检查是否已终结
    #[inline]
    pub fn is_finalized(&self) -> bool {
        (self.marked.get() & bits::FINALIZED) != 0
    }
}

// Safety: GcObjectHeader is only accessed through GC-managed pointers,
// and interior mutability via Cell is thread-safe for single-threaded use.
// Not Send/Sync because raw pointers are involved.
impl std::fmt::Debug for GcObjectHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcObjectHeader")
            .field("next", &self.next.get())
            .field("marked", &self.marked.get())
            .field("gc_type", &self.gc_type)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::GcColor;

    #[test]
    fn test_header_new_defaults() {
        let h = GcObjectHeader::new(GcObjectType::String);
        assert_eq!(h.gc_type(), GcObjectType::String);
        assert_eq!(h.marked(), 0);
        assert!(h.next().is_null());
        // marked=0 → neither WHITEBITS nor BLACK set → Gray
        // (matches C++ behavior: fresh GCObject::marked_ = 0)
        assert!(h.is_gray());
    }

    #[test]
    fn test_color_transitions() {
        let h = GcObjectHeader::new(GcObjectType::Table);
        // Default: marked=0, WHITE0|WHITE1 bits are 0, BLACK is 0 → this is Gray!
        assert!(h.is_gray());

        // Set to white
        h.set_color(GcColor::White);
        assert!(h.is_white());
        assert!(!h.is_gray());
        assert!(!h.is_black());

        // Set to gray
        h.set_color(GcColor::Gray);
        assert!(!h.is_white());
        assert!(h.is_gray());
        assert!(!h.is_black());

        // Set to black
        h.set_color(GcColor::Black);
        assert!(!h.is_white());
        assert!(!h.is_gray());
        assert!(h.is_black());
    }

    #[test]
    fn test_fixed_bit() {
        let h = GcObjectHeader::new(GcObjectType::String);
        assert!(!h.is_fixed());
        h.mark_fixed();
        assert!(h.is_fixed());
        // FIXED bit should not affect color
        h.set_color(GcColor::Black);
        assert!(h.is_black());
        assert!(h.is_fixed());
    }

    #[test]
    fn test_linked_list_chain() {
        let a = GcObjectHeader::new(GcObjectType::String);
        let b = GcObjectHeader::new(GcObjectType::Table);
        a.set_next(&b as *const _ as *mut _);
        assert!(!a.next().is_null());
        assert_eq!(unsafe { (*a.next()).gc_type() }, GcObjectType::Table);
    }

    #[test]
    fn test_gc_bits_constants() {
        assert_eq!(bits::WHITE0, 1);
        assert_eq!(bits::WHITE1, 2);
        assert_eq!(bits::BLACK, 4);
        assert_eq!(bits::WHITEBITS, 3);
        assert_eq!(bits::FIXED, 0x20);
        assert_eq!(bits::WEAKBITS, 0x50);
    }
}
