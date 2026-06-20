//! Lua 线程状态 (LuaState)
//!
//! 每个 LuaState 代表一个独立的协程执行环境，包含值栈、调用栈、
//! 全局表引用和线程状态。
//!
//! C++ 参考: `lua_cpp/src/vm/state/lua_state.hpp`

use lua_core::value::Value;

use super::call_info::CallInfo;
use super::stack::Stack;

/// 线程执行状态
///
/// C++ 对应: `Lua::ThreadStatus`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ThreadStatus {
    Ok = 0,
    Yield = 1,
    ErrRun = 2,
    ErrSyntax = 3,
    ErrMem = 4,
    ErrErr = 5,
}

/// Lua 线程状态
///
/// 管理单个 Lua 线程（协程）的完整执行环境。
///
/// C++ 对应: `Lua::LuaState`
#[derive(Debug)]
pub struct LuaState {
    /// 值栈
    pub stack: Stack,
    /// 栈顶索引
    pub top: usize,
    /// 调用信息栈
    pub call_stack: Vec<CallInfo>,
    /// 当前调用信息索引
    pub current_ci: usize,
    /// 线程状态
    pub status: ThreadStatus,
    /// 调用嵌套深度
    pub nccalls: i32,
    /// allow yield counter
    pub allow_yield: u16,
}

impl LuaState {
    /// 创建新的 Lua 线程
    pub fn new() -> Self {
        let stack = Stack::with_default();
        Self {
            stack,
            top: 0,
            call_stack: vec![CallInfo::new()],
            current_ci: 0,
            status: ThreadStatus::Ok,
            nccalls: 1,
            allow_yield: 0,
        }
    }

    // ── 栈操作 ──────────────────────────────────────────────────

    /// 获取栈大小（元素计数）
    pub fn get_top(&self) -> i32 {
        self.top as i32
    }

    /// 设置栈大小
    pub fn set_top(&mut self, idx: i32) {
        let new_top = if idx >= 0 { idx as usize } else { 0 };
        self.top = new_top;
    }

    /// 压入 nil
    pub fn push_nil(&mut self) {
        self.push_value(Value::Nil);
    }

    /// 压入布尔值
    pub fn push_boolean(&mut self, b: bool) {
        self.push_value(Value::Boolean(b));
    }

    /// 压入数值
    pub fn push_number(&mut self, n: f64) {
        self.push_value(Value::Number(n));
    }

    /// 通用值压入
    pub fn push_value(&mut self, v: Value) {
        if self.top >= self.stack.capacity() {
            self.stack.ensure_space(32);
        }
        while self.stack.size() <= self.top {
            self.stack.push(Value::Nil);
        }
        // We use the stack's internal buffer directly
        if let Some(slot) = self.stack.at_mut(self.top) {
            *slot = v;
        }
        self.top += 1;
    }

    /// 弹出栈顶值
    pub fn pop(&mut self) -> Option<Value> {
        if self.top == 0 {
            return None;
        }
        self.top -= 1;
        self.stack.at(self.top).cloned()
    }

    /// 按绝对索引获取值
    pub fn at_abs(&self, idx: usize) -> Option<&Value> {
        self.stack.at(idx)
    }

    // ── 调用信息管理 ────────────────────────────────────────────

    /// 获取当前 CallInfo
    pub fn current_call_info(&self) -> &CallInfo {
        &self.call_stack[self.current_ci]
    }

    /// 获取当前 CallInfo（可写）
    pub fn current_call_info_mut(&mut self) -> &mut CallInfo {
        &mut self.call_stack[self.current_ci]
    }

    /// 压入新的 CallInfo（函数调用前）
    pub fn push_call_info(&mut self) -> &mut CallInfo {
        if self.current_ci + 1 >= self.call_stack.len() {
            self.call_stack.push(CallInfo::new());
        }
        self.current_ci += 1;
        &mut self.call_stack[self.current_ci]
    }

    /// 弹出当前 CallInfo（函数返回后）
    pub fn pop_call_info(&mut self) {
        if self.current_ci > 0 {
            self.current_ci -= 1;
        }
    }

    /// 调用栈大小
    pub fn call_stack_size(&self) -> usize {
        self.current_ci + 1
    }

    // ── 线程状态 ────────────────────────────────────────────────

    pub fn get_status(&self) -> ThreadStatus {
        self.status
    }

    pub fn set_status(&mut self, s: ThreadStatus) {
        self.status = s;
    }
}

impl Default for LuaState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lua_state_new() {
        let ls = LuaState::new();
        assert_eq!(ls.get_status(), ThreadStatus::Ok);
        assert_eq!(ls.get_top(), 0);
        assert_eq!(ls.call_stack_size(), 1);
    }

    #[test]
    fn test_push_pop() {
        let mut ls = LuaState::new();
        ls.push_number(42.0);
        ls.push_boolean(true);
        assert_eq!(ls.get_top(), 2);
        assert_eq!(ls.pop(), Some(Value::Boolean(true)));
        assert_eq!(ls.pop(), Some(Value::Number(42.0)));
        assert_eq!(ls.pop(), None);
    }

    #[test]
    fn test_call_info() {
        let mut ls = LuaState::new();
        let ci = ls.push_call_info();
        ci.func = 10;
        ci.base = 11;
        assert_eq!(ls.current_call_info().func, 10);
        ls.pop_call_info();
        assert_eq!(ls.current_call_info().func, 0);
    }
}
