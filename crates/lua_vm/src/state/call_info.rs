//! 调用帧信息 (CallInfo)
//!
//! 存储单次函数调用的上下文：函数位置、栈基址、PC、返回值期望等。
//!
//! 栈帧示意图（栈索引向右增长）：
//!
//! ```text
//! 调用前/调用中：
//!   caller frame ... | func | arg0 | arg1 | ... | local/temp ... | reserved |
//!                       ^      ^                              ^             ^
//!                       |      |                              |             |
//!                    ci.func ci.base                      lua_State.top   ci.top
//!
//! 关系：
//!   - ci.func：被调用函数对象所在的栈槽。
//!   - ci.base：当前函数寄存器/参数区域起点，通常等于 ci.func + 1。
//!   - ci.top：当前调用帧可使用栈空间的上界（不含该位置）。
//!   - lua_State.top：当前实际使用到的栈顶（不含该位置），会在 ci.top 范围内移动。
//!
//! 返回后：
//!   caller frame ... | ret0 | ret1 | ...
//!                       ^
//!                       |
//!                    原 ci.func（返回值从这里覆盖调用表达式）
//! ```
//!

use lua_core::proto::Proto;
use lua_core::value::Value;

/// 常量：接受所有返回值
pub const LUA_MULTRET: i32 = -1;

/// 调用帧信息
///
/// 存储单次函数调用的完整上下文。
///
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
    /// 当前 Lua 调用帧对应的函数原型。
    pub proto: Option<*const Proto>,
    /// 期望返回值数量（-1 = LUA_MULTRET）
    pub nresults: i32,
    /// 实际传入参数数量
    pub nargs: i32,
    /// 可变参数快照（不受局部变量覆盖实参槽影响）
    pub varargs: Vec<Value>,
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
            proto: None,
            nresults: 0,
            nargs: 0,
            varargs: Vec::new(),
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
            proto: None,
            nresults: 0,
            nargs: 0,
            varargs: Vec::new(),
            tailcalls: 0,
        }
    }

    pub fn reset(&mut self) {
        self.func = 0;
        self.base = 0;
        self.top = 0;
        self.savedpc = None;
        self.proto = None;
        self.nresults = 0;
        self.nargs = 0;
        self.varargs.clear();
        self.tailcalls = 0;
    }
}

impl Default for CallInfo {
    fn default() -> Self {
        Self::new()
    }
}
