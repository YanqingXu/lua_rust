//! Lua 线程状态 (LuaState)
//!
//! 每个 LuaState 代表一个独立的协程执行环境，包含值栈、调用栈、
//! 全局表引用和线程状态。
//!

use std::collections::HashMap;

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::thread::Thread;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use super::call_info::CallInfo;
use super::stack::Stack;

/// 线程执行状态
///
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
    /// 全局表 (_G)
    pub global_table: Option<GcRef<Table>>,
    /// Thread environment used by getfenv/setfenv level 0 and loaded chunks.
    pub thread_env: Option<GcRef<Table>>,
    /// Environment for the currently executing top-level chunk pseudo-frame.
    pub chunk_env: Option<GcRef<Table>>,
    /// Coroutine object that owns this state, if any.
    pub current_thread: Option<GcRef<Thread>>,
    /// Optional metatable for nil values, configured through debug.setmetatable.
    pub nil_metatable: Option<GcRef<Table>>,
    /// Optional metatable for boolean values, configured through debug.setmetatable.
    pub boolean_metatable: Option<GcRef<Table>>,
    /// Optional metatable for number values, configured through debug.setmetatable.
    pub number_metatable: Option<GcRef<Table>>,
    /// 字符串驻留池（用于跨编译器和标准库的字符串共享）
    pub string_pool: Option<*mut StringPool>,
    /// 当前运行时 GC（供 C 函数创建返回字符串/表等 GC 对象）
    pub gc: Option<*mut GarbageCollector>,
    /// Proto currently executing in the VM loop; used by C helpers called from a
    /// top-level chunk that has no Function object in its call frame.
    pub current_proto: Option<*const Proto>,
    /// Active debug hook function configured through debug.sethook.
    pub debug_hook: Option<Value>,
    /// Debug hook event mask, e.g. "crl".
    pub debug_hook_mask: String,
    /// Instruction-count hook interval.
    pub debug_hook_count: i32,
    /// Countdown until the next count hook event.
    pub debug_hook_countdown: i32,
    /// Guard against recursively invoking hooks from hook code.
    pub debug_hook_active: bool,
    /// Last source line reported to a line hook.
    pub debug_hook_last_line: i32,
    /// Last program counter reported to a line hook.
    pub debug_hook_last_pc: usize,
    /// Caller Proto whose current source line should be skipped immediately after sethook.
    pub debug_hook_skip_proto: Option<*const Proto>,
    /// Caller source line to skip immediately after sethook.
    pub debug_hook_skip_line: i32,
    /// Cache for debug.getinfo(..., "n") call-site name lookups.
    pub debug_name_cache: HashMap<(usize, usize, usize), (Option<String>, String)>,
    /// Open upvalue 链表头（按栈索引降序排列）
    pub open_upvalues: Option<GcRef<Upvalue>>,
    /// Values passed out by the most recent coroutine.yield.
    pub yielded_values: Vec<Value>,
    /// Register where resume arguments should be written when a yield call resumes.
    pub yield_result_base: Option<usize>,
    /// Number of results expected by the suspended yield call.
    pub yield_wanted_results: Option<usize>,
    /// Last runtime error value that terminated this coroutine.
    pub last_error: Option<Value>,
    /// Compatibility counter used by Lua 5.1 gcinfo/collectgarbage probes.
    pub gcinfo_kb: f64,
    /// Whether automatic GC progress is stopped by collectgarbage("stop").
    pub gc_stopped: bool,
    /// Poll count for simulated automatic GC progress.
    pub gcinfo_polls: usize,
    /// Remaining steps in the current collectgarbage("step") cycle.
    pub gc_step_remaining: i32,
    /// Countdown for lightweight automatic weak-table cleanup.
    pub auto_gc_countdown: i32,
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
            global_table: None,
            thread_env: None,
            chunk_env: None,
            current_thread: None,
            nil_metatable: None,
            boolean_metatable: None,
            number_metatable: None,
            string_pool: None,
            gc: None,
            current_proto: None,
            debug_hook: None,
            debug_hook_mask: String::new(),
            debug_hook_count: 0,
            debug_hook_countdown: 0,
            debug_hook_active: false,
            debug_hook_last_line: -1,
            debug_hook_last_pc: usize::MAX,
            debug_hook_skip_proto: None,
            debug_hook_skip_line: -1,
            debug_name_cache: HashMap::new(),
            open_upvalues: None,
            yielded_values: Vec::new(),
            yield_result_base: None,
            yield_wanted_results: None,
            last_error: None,
            gcinfo_kb: 128.0,
            gc_stopped: false,
            gcinfo_polls: 0,
            gc_step_remaining: 0,
            auto_gc_countdown: 0,
        }
    }

    /// 创建带全局表的 Lua 线程
    pub fn with_global_table(global: GcRef<Table>) -> Self {
        let mut state = Self::new();
        state.global_table = Some(global);
        state.thread_env = Some(global);
        state.chunk_env = Some(global);
        state
    }

    // ── 栈操作 ──────────────────────────────────────────────────

    /// 获取栈大小（元素计数）
    pub fn get_top(&self) -> i32 {
        self.top.saturating_sub(self.current_call_info().base) as i32
    }

    /// 设置栈大小
    pub fn set_top(&mut self, idx: i32) {
        let base = self.current_call_info().base;
        let new_top = if idx >= 0 {
            base + idx as usize
        } else {
            self.top.saturating_sub((-idx) as usize)
        };
        self.stack.set_top(new_top);
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
        if self.top <= self.current_call_info().base {
            return None;
        }
        self.top -= 1;
        self.stack.at(self.top).cloned()
    }

    /// 将 Lua C API 风格索引转换为绝对栈索引。
    ///
    /// 正数索引从当前 CallInfo base 开始（1 表示第一个参数），负数
    /// 索引从当前 top 向后计数（-1 表示栈顶）。
    pub fn abs_index(&self, idx: i32) -> Option<usize> {
        if idx > 0 {
            let abs = self.current_call_info().base + idx as usize - 1;
            if abs < self.top { Some(abs) } else { None }
        } else if idx < 0 {
            let offset = (-idx) as usize;
            let base = self.current_call_info().base;
            if offset <= self.top.saturating_sub(base) {
                Some(self.top - offset)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// 按当前调用帧的相对索引获取值。
    pub fn at(&self, idx: i32) -> Option<&Value> {
        self.abs_index(idx).and_then(|abs| self.stack.at(abs))
    }

    /// 按当前调用帧的相对索引获取可写值。
    pub fn at_mut(&mut self, idx: i32) -> Option<&mut Value> {
        self.abs_index(idx).and_then(|abs| self.stack.at_mut(abs))
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

    // ── Upvalue 管理 ────────────────────────────────────────────

    pub fn find_or_create_upvalue(
        &mut self,
        stack_index: usize,
        gc: &mut GarbageCollector,
    ) -> GcRef<Upvalue> {
        let mut prev: Option<GcRef<Upvalue>> = None;
        let mut curr = self.open_upvalues;

        while let Some(curr_ref) = curr {
            // SAFETY: open_upvalues only contains live Upvalue refs allocated by GC.
            let curr_uv = unsafe { curr_ref.as_ref() }.expect("open upvalue should be valid");
            if curr_uv.stack_index_any() <= stack_index {
                break;
            }
            prev = curr;
            curr = curr_uv.next();
        }

        if let Some(curr_ref) = curr {
            // SAFETY: same as above.
            let curr_uv = unsafe { curr_ref.as_ref() }.expect("open upvalue should be valid");
            if curr_uv.stack_index_any() == stack_index {
                return curr_ref;
            }
        }

        let stack_ptr = &mut self.stack as *mut Stack as *mut std::ffi::c_void;
        let new_ref = gc.create(Upvalue::new_open(stack_index, stack_ptr));
        // SAFETY: new_ref is freshly allocated and uniquely reachable here.
        unsafe {
            (*(new_ref.as_ptr() as *mut Upvalue)).set_next(curr);
        }

        if let Some(prev_ref) = prev {
            // SAFETY: prev_ref is a live upvalue in the open list.
            unsafe {
                (*(prev_ref.as_ptr() as *mut Upvalue)).set_next(Some(new_ref));
            }
        } else {
            self.open_upvalues = Some(new_ref);
        }

        new_ref
    }

    pub fn close_upvalues(&mut self, level: usize) {
        while let Some(uv_ref) = self.open_upvalues {
            // SAFETY: open_upvalues only contains live Upvalue refs allocated by GC.
            let stack_index = unsafe { uv_ref.as_ref() }
                .expect("open upvalue should be valid")
                .stack_index_any();
            if stack_index < level {
                break;
            }

            // SAFETY: uv_ref is the current head of the open_upvalues list and
            // therefore points to a live Upvalue allocated by GC.
            let next = unsafe { uv_ref.as_ref() }
                .expect("open upvalue should be valid")
                .next();
            self.open_upvalues = next;
            let value = self.stack.at(stack_index).cloned().unwrap_or(Value::Nil);
            // SAFETY: uv_ref is being removed from the open list and remains live
            // through any closures that captured it.
            unsafe {
                let uv = &mut *(uv_ref.as_ptr() as *mut Upvalue);
                uv.close(value);
                uv.set_next(None);
            }
        }
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
