//! 调用帧信息 (CallInfo)
//!
//! 存储单次函数调用的上下文：函数位置、栈基址、PC、返回值期望等。
//!
//! C++ 参考: `lua_cpp/src/vm/state/call_info.hpp`

/// 常量：接受所有返回值
pub const LUA_MULTRET: i32 = -1;

/// 调用帧信息
///
/// 存储单次函数调用的完整上下文。
///
/// C++ 对应: `Lua::CallInfo`
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// 函数对象在栈中的索引
    pub func: usize,
    /// 栈基址（参数和局部变量的起始位置）
    pub base: usize,
    /// 栈顶（可用栈空间的上界）
    pub top: usize,
    /// 保存的程序计数器（Lua 函数指向当前指令，C 函数为 None）
    pub savedpc: Option<usize>,
    /// 期望返回值数量（-1 = LUA_MULTRET）
    pub nresults: i32,
    /// 尾调用计数
    pub tailcalls: i32,
}

impl CallInfo {
    pub fn new() -> Self {
        Self {
            func: 0,
            base: 0,
            top: 0,
            savedpc: None,
            nresults: 0,
            tailcalls: 0,
        }
    }

    /// 创建带指定栈帧的 CallInfo
    pub fn with_frame(func: usize, base: usize, top: usize) -> Self {
        Self {
            func,
            base,
            top,
            savedpc: None,
            nresults: 0,
            tailcalls: 0,
        }
    }

    pub fn reset(&mut self) {
        self.func = 0;
        self.base = 0;
        self.top = 0;
        self.savedpc = None;
        self.nresults = 0;
        self.tailcalls = 0;
    }
}

impl Default for CallInfo {
    fn default() -> Self {
        Self::new()
    }
}
