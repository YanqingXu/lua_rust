//! 寄存器分配器
//!
//! 管理临时寄存器的分配/回收和 maxStackSize 维护。
//!
//! C++ 参考: `lua_cpp/src/compiler/register_allocator.hpp`

/// 寄存器分配器
///
/// 负责临时寄存器的分配/回收，以及 maxStackSize 的维护。
///
/// C++ 对应: `Lua::RegisterAllocator`
#[derive(Debug)]
pub struct RegisterAllocator {
    // 下一个空闲寄存器
    freereg: i32,
}

impl RegisterAllocator {
    pub fn new(start: i32) -> Self {
        Self { freereg: start }
    }

    /// 当前下一个空闲寄存器
    pub fn current(&self) -> i32 {
        self.freereg
    }

    /// 分配一个新寄存器
    pub fn alloc(&mut self) -> i32 {
        let reg = self.freereg;
        self.freereg += 1;
        reg
    }

    /// 分配 n 个连续寄存器，返回起始寄存器
    pub fn alloc_n(&mut self, n: i32) -> i32 {
        let reg = self.freereg;
        self.freereg += n;
        reg
    }

    /// 尝试回收栈顶寄存器
    pub fn free_reg(&mut self, reg: i32, active_locals: i32) {
        if reg >= active_locals && reg == self.freereg - 1 {
            self.freereg -= 1;
        }
    }

    /// 回收栈顶 n 个寄存器
    pub fn free_regs(&mut self, n: i32) {
        self.freereg -= n;
    }

    /// 检查是否需要更新 maxStackSize
    ///
    /// 返回新的 stack 大小（调用者应更新 Proto::maxStackSize）
    pub fn check_stack(&mut self, n: i32, current_max: i32) -> i32 {
        let newstack = self.freereg + n;
        if newstack > current_max {
            newstack
        } else {
            current_max
        }
    }

    /// 将下一个空闲寄存器设置到指定位置
    pub fn set_freereg(&mut self, reg: i32) {
        self.freereg = reg;
    }

    /// 将下一个空闲寄存器重置到当前活动局部变量之后
    pub fn reset_to_locals(&mut self, active_locals: i32) {
        self.freereg = active_locals;
    }

    /// 恢复到先前保存的空闲寄存器位置
    pub fn restore(&mut self, saved: i32) {
        self.freereg = saved;
    }

    /// 保留连续寄存器，不立即更新 maxStackSize
    pub fn reserve(&mut self, count: i32) {
        self.freereg += count;
    }

    /// 确保下一个空闲寄存器至少位于指定位置
    pub fn ensure_at_least(&mut self, reg: i32) {
        if self.freereg < reg {
            self.freereg = reg;
        }
    }

    /// 重置到初始状态（用于子函数编译）
    pub fn reset(&mut self, start: i32) {
        self.freereg = start;
    }
}

impl Default for RegisterAllocator {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alloc_free() {
        let mut ra = RegisterAllocator::new(0);
        assert_eq!(ra.current(), 0);

        let r1 = ra.alloc();
        assert_eq!(r1, 0);
        assert_eq!(ra.current(), 1);

        let r2 = ra.alloc();
        assert_eq!(r2, 1);
        assert_eq!(ra.current(), 2);

        // 回收栈顶寄存器
        ra.free_reg(r2, 0);
        assert_eq!(ra.current(), 1);

        // r1 is now the top (after r2 was freed), so it can also be freed
        ra.free_reg(r1, 0);
        assert_eq!(ra.current(), 0);
    }

    #[test]
    fn test_free_non_top_register() {
        let mut ra = RegisterAllocator::new(0);
        let _r1 = ra.alloc(); // r1=0
        let r2 = ra.alloc(); // r2=1
        let _r3 = ra.alloc(); // r3=2

        // Try to free r2 when r3 is still at top — should not decrement
        ra.free_reg(r2, 0);
        assert_eq!(ra.current(), 3); // unchanged
    }

    #[test]
    fn test_alloc_n() {
        let mut ra = RegisterAllocator::new(0);
        let base = ra.alloc_n(5);
        assert_eq!(base, 0);
        assert_eq!(ra.current(), 5);
    }

    #[test]
    fn test_check_stack() {
        let mut ra = RegisterAllocator::new(3);
        let max = ra.check_stack(5, 4);
        assert_eq!(max, 8); // freereg(3) + n(5) = 8 > current_max(4)
    }

    #[test]
    fn test_ensure_at_least() {
        let mut ra = RegisterAllocator::new(2);
        ra.ensure_at_least(10);
        assert_eq!(ra.current(), 10);

        ra.ensure_at_least(5);
        assert_eq!(ra.current(), 10); // unchanged
    }
}
