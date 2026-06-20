//! Lua 协程/线程对象
//!
//! `Thread` 是 GC 管理的协程对象，每个 Thread 持有一个独立的 LuaState
//!（独立栈 + 调用栈，共享 GlobalState）。
//!
//! ## 核心设计
//! - 所有执行现场保存在 LuaState/CallInfo 中（不依赖宿主栈帧）
//! - VM 通过 `ExecResult::Yielded` 退出执行循环
//! - `resume`/`yield` 通过显式的值搬运 + VM 重入实现
//!
//! ## Phase 1.4 限制
//! LuaState 在 Phase 3 才实现，当前 `state_` 和 `callerState_` 使用不透明
//! 指针占位。`mark_children` 仅标记 `caller` 线程；栈内容的标记将在
//! Phase 3 通过 `gc.markState()` 启用。
//!
//! C++ 参考: `lua_cpp/src/core/thread.hpp`, `lua_cpp/src/core/thread.cpp`

use crate::gc::collector::GarbageCollector;
use crate::gc::gc_object::GcObject;
use crate::gc::gc_ref::GcRef;
use crate::gc::header::GcObjectHeader;
use crate::types::GcObjectType;

// =====================================================================
// CoroutineStatus 枚举
// =====================================================================

/// Lua 协程状态（与 C++ `ThreadStatus` 不同，这是 Lua 层面语义）
///
/// C++ 对应: `Lua::CoroutineStatus` (enum class : u8)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CoroutineStatus {
    /// 创建后 / yield 后
    Suspended = 0,
    /// 正在执行
    Running = 1,
    /// resume 了其他协程，自身暂停
    Normal = 2,
    /// 函数返回或出错
    Dead = 3,
}

impl std::fmt::Display for CoroutineStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoroutineStatus::Suspended => write!(f, "suspended"),
            CoroutineStatus::Running => write!(f, "running"),
            CoroutineStatus::Normal => write!(f, "normal"),
            CoroutineStatus::Dead => write!(f, "dead"),
        }
    }
}

// =====================================================================
// Thread 结构体
// =====================================================================

/// Lua 线程/协程对象
///
/// Thread 是 GC 管理的协程对象，持有独立的执行状态。
///
/// 内存布局（`#[repr(C)]`，header 在开头）：
/// - header: GcObjectHeader (16 bytes)
/// - state: *mut c_void (8 bytes) — Phase 3 具体化为 `*mut LuaState`
/// - co_status: CoroutineStatus (1 byte)
/// - first_resume: bool (1 byte)
/// - padding: (6 bytes)
/// - caller: Option<GcRef<Thread>> (8 bytes)
/// - caller_state: *mut c_void (8 bytes) — Phase 3 具体化为 `*mut LuaState`
/// - saved_nexeccalls: i32 (4 bytes)
/// - padding: (4 bytes)
///   总计约 56+ bytes
///
/// C++ 对应: `Lua::Thread`（继承 `GCObject`）
#[repr(C)]
pub struct Thread {
    /// GC 对象头部（必须在结构体开头）
    header: GcObjectHeader,

    /// LuaState 指针
    ///
    /// Phase 1.4: 不透明占位（`*mut c_void`）。
    /// Phase 3: 具体化为 `*mut LuaState`。
    ///
    /// C++ 对应: `Thread::state_`（`UPtr<LuaState>`）
    state: *mut std::ffi::c_void,

    /// 协程状态
    ///
    /// C++ 对应: `Thread::coStatus_`（`CoroutineStatus`）
    co_status: CoroutineStatus,

    /// 是否为首次 resume
    ///
    /// C++ 对应: `Thread::firstResume_`
    first_resume: bool,

    /// Resume 链：调用当前协程的协程
    ///
    /// C++ 对应: `Thread::caller_`（`Thread*`）
    caller: Option<GcRef<Thread>>,

    /// Resume 链：调用者的 LuaState
    ///
    /// Phase 1.4: 不透明占位。
    /// Phase 3: 具体化为 `*mut LuaState`。
    ///
    /// C++ 对应: `Thread::callerState_`（`LuaState*`）
    caller_state: *mut std::ffi::c_void,

    /// VM 重入保护：保存的嵌套执行计数
    ///
    /// C++ 对应: `Thread::savedNexeccalls_`
    saved_nexeccalls: i32,
}

impl Thread {
    /// 创建新的协程
    ///
    /// 初始状态为 `Suspended`，LuaState 指针为空（Phase 3 初始化）。
    ///
    /// C++ 对应: `Thread::Thread(UPtr<LuaState> state)` + `Thread::create()`
    pub fn new() -> Self {
        Self {
            header: GcObjectHeader::new(GcObjectType::Thread),
            state: std::ptr::null_mut(),
            co_status: CoroutineStatus::Suspended,
            first_resume: true,
            caller: None,
            caller_state: std::ptr::null_mut(),
            saved_nexeccalls: 1,
        }
    }

    // ── 状态查询 ──────────────────────────────────────────────────

    /// 获取协程状态
    #[inline]
    pub fn status(&self) -> CoroutineStatus {
        self.co_status
    }

    /// 设置协程状态
    #[inline]
    pub fn set_status(&mut self, status: CoroutineStatus) {
        self.co_status = status;
    }

    /// 检查协程是否已终止
    #[inline]
    pub fn is_dead(&self) -> bool {
        self.co_status == CoroutineStatus::Dead
    }

    /// 检查协程是否挂起（可被 resume）
    #[inline]
    pub fn is_suspended(&self) -> bool {
        self.co_status == CoroutineStatus::Suspended
    }

    /// 检查协程是否正在运行
    #[inline]
    pub fn is_running(&self) -> bool {
        self.co_status == CoroutineStatus::Running
    }

    // ── LuaState 访问（占位）──────────────────────────────────────

    /// 获取 LuaState 指针（不透明）
    ///
    /// Phase 1.4: 返回不透明指针。
    /// Phase 3: 具体化为 `*mut LuaState`。
    ///
    /// C++ 对应: `Thread::getLuaState() const`
    #[inline]
    pub fn lua_state(&self) -> *mut std::ffi::c_void {
        self.state
    }

    /// 设置 LuaState 指针
    ///
    /// Phase 3 在创建 Thread 时调用，关联新创建的 LuaState。
    #[inline]
    pub fn set_lua_state(&mut self, state: *mut std::ffi::c_void) {
        self.state = state;
    }

    // ── Resume 链管理 ─────────────────────────────────────────────

    /// 获取调用者协程
    ///
    /// C++ 对应: `Thread::getCaller() const`
    #[inline]
    pub fn caller(&self) -> Option<GcRef<Thread>> {
        self.caller
    }

    /// 设置调用者协程
    ///
    /// C++ 对应: `Thread::setCaller(Thread* t)`
    pub fn set_caller(&mut self, caller: Option<GcRef<Thread>>) {
        // TODO Phase 1.3+: write barrier — gc->writeBarrier(this, caller)
        self.caller = caller;
    }

    /// 获取调用者的 LuaState（不透明）
    ///
    /// Phase 1.4: 返回不透明指针。
    /// Phase 3: 具体化为 `*mut LuaState`。
    #[inline]
    pub fn caller_state(&self) -> *mut std::ffi::c_void {
        self.caller_state
    }

    /// 设置调用者的 LuaState
    #[inline]
    pub fn set_caller_state(&mut self, state: *mut std::ffi::c_void) {
        self.caller_state = state;
    }

    // ── 首次 Resume 标志 ──────────────────────────────────────────

    /// 是否为首次 resume
    #[inline]
    pub fn is_first_resume(&self) -> bool {
        self.first_resume
    }

    /// 标记已完成首次 resume
    #[inline]
    pub fn mark_resumed(&mut self) {
        self.first_resume = false;
    }

    // ── 嵌套执行计数 ──────────────────────────────────────────────

    /// 获取保存的嵌套执行计数
    #[inline]
    pub fn saved_nexeccalls(&self) -> i32 {
        self.saved_nexeccalls
    }

    /// 设置保存的嵌套执行计数
    #[inline]
    pub fn set_saved_nexeccalls(&mut self, n: i32) {
        self.saved_nexeccalls = n;
    }
}

impl Default for Thread {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// GcObject trait 实现
// =====================================================================

// SAFETY: Thread 以 GcObjectHeader 开头 (#[repr(C)])，
// gc_type 在构造时正确设置为 GcObjectType::Thread。
// mark_children 标记 caller 链中的协程。
unsafe impl GcObject for Thread {
    fn gc_header(&self) -> &GcObjectHeader {
        &self.header
    }

    fn gc_header_mut(&mut self) -> &mut GcObjectHeader {
        &mut self.header
    }

    /// 标记 Thread 引用的 GC 对象
    ///
    /// Phase 1.4: 标记 caller 协程。
    /// Phase 3: 还将通过 `gc.markState()` 标记 LuaState 栈上的所有值。
    ///
    /// C++ 对应: `Thread::mark(GarbageCollector& gc)`
    unsafe fn mark_children(&self, collector: &mut GarbageCollector) {
        // 标记 caller 协程
        if let Some(caller_ref) = self.caller {
            // SAFETY: caller_ref is a valid GcRef<Thread> held by this Thread;
            // collector is valid during mark phase.
            unsafe {
                collector.mark_object(caller_ref.as_ptr() as *mut GcObjectHeader);
            }
        }
        // Phase 3: gc.markState(state_) 和 gc.markState(callerState_)
    }

    fn get_size(&self) -> usize {
        // Phase 1.4: 返回基础大小。
        // Phase 3: 加上 LuaState 的栈和调用栈容量。
        std::mem::size_of::<Self>()
    }
}

// =====================================================================
// Debug
// =====================================================================

impl std::fmt::Debug for Thread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thread")
            .field("status", &self.co_status)
            .field("first_resume", &self.first_resume)
            .field("has_caller", &self.caller.is_some())
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
    use crate::gc::gc_ref::GcRef;
    use crate::string_pool::StringPool;

    // ── 创建与默认状态 ────────────────────────────────────────────

    #[test]
    fn test_thread_new_defaults() {
        let t = Thread::new();
        assert_eq!(t.status(), CoroutineStatus::Suspended);
        assert!(t.is_suspended());
        assert!(!t.is_dead());
        assert!(!t.is_running());
        assert!(t.is_first_resume());
        assert_eq!(t.saved_nexeccalls(), 1);
        assert!(t.lua_state().is_null());
        assert!(t.caller().is_none());
        assert!(t.caller_state().is_null());
    }

    #[test]
    fn test_thread_default() {
        let t = Thread::default();
        assert_eq!(t.status(), CoroutineStatus::Suspended);
    }

    // ── 状态转换 ──────────────────────────────────────────────────

    #[test]
    fn test_thread_status_transitions() {
        let mut t = Thread::new();

        t.set_status(CoroutineStatus::Running);
        assert!(t.is_running());
        assert!(!t.is_suspended());

        t.set_status(CoroutineStatus::Normal);
        assert_eq!(t.status(), CoroutineStatus::Normal);

        t.set_status(CoroutineStatus::Dead);
        assert!(t.is_dead());
    }

    #[test]
    fn test_thread_status_display() {
        assert_eq!(format!("{}", CoroutineStatus::Suspended), "suspended");
        assert_eq!(format!("{}", CoroutineStatus::Running), "running");
        assert_eq!(format!("{}", CoroutineStatus::Normal), "normal");
        assert_eq!(format!("{}", CoroutineStatus::Dead), "dead");
    }

    // ── 首次 Resume 标志 ──────────────────────────────────────────

    #[test]
    fn test_first_resume_flag() {
        let mut t = Thread::new();
        assert!(t.is_first_resume());

        t.mark_resumed();
        assert!(!t.is_first_resume());
    }

    // ── 嵌套执行计数 ──────────────────────────────────────────────

    #[test]
    fn test_saved_nexeccalls() {
        let mut t = Thread::new();
        assert_eq!(t.saved_nexeccalls(), 1);

        t.set_saved_nexeccalls(5);
        assert_eq!(t.saved_nexeccalls(), 5);
    }

    // ── LuaState 占位 ─────────────────────────────────────────────

    #[test]
    fn test_lua_state_placeholder() {
        let mut t = Thread::new();
        assert!(t.lua_state().is_null());

        let dummy: *mut std::ffi::c_void = 0x1000 as *mut _;
        t.set_lua_state(dummy);
        assert_eq!(t.lua_state(), dummy);
    }

    // ── Caller 链管理 ─────────────────────────────────────────────

    #[test]
    fn test_caller_chain() {
        let mut gc = GarbageCollector::new();
        let caller_thread = gc.create(Thread::new());
        let mut callee = Thread::new();

        assert!(callee.caller().is_none());

        callee.set_caller(Some(caller_thread));
        assert_eq!(callee.caller(), Some(caller_thread));

        callee.set_caller(None);
        assert!(callee.caller().is_none());
    }

    #[test]
    fn test_caller_state_placeholder() {
        let mut t = Thread::new();
        assert!(t.caller_state().is_null());

        let dummy: *mut std::ffi::c_void = 0x2000 as *mut _;
        t.set_caller_state(dummy);
        assert_eq!(t.caller_state(), dummy);
    }

    // ── GC 类型测试 ───────────────────────────────────────────────

    #[test]
    fn test_thread_gc_header_type() {
        let t = Thread::new();
        assert_eq!(t.gc_header().gc_type(), GcObjectType::Thread);
    }

    #[test]
    fn test_thread_gc_create_and_register() {
        let mut gc = GarbageCollector::new();
        let t = Thread::new();
        let t_ref: GcRef<Thread> = gc.create(t);

        assert!(!t_ref.is_null());
        assert_eq!(gc.object_count(), 1);
    }

    // ── GC 标记测试 ───────────────────────────────────────────────

    #[test]
    fn test_thread_mark_caller() {
        let mut gc = GarbageCollector::new();

        let caller = gc.create(Thread::new());
        let mut callee = Thread::new();
        callee.set_caller(Some(caller));
        let callee_ref = gc.create(callee);

        gc.reset_marks();

        // 标记 callee → 应标记 caller
        unsafe {
            let t_ptr = callee_ref.as_ptr();
            (*t_ptr).mark_children(&mut gc);
        }

        let caller_header = caller.as_ptr() as *mut GcObjectHeader;
        unsafe {
            assert!(
                !(*caller_header).is_white(),
                "Caller thread should be marked"
            );
        }
    }

    #[test]
    fn test_thread_mark_no_caller() {
        let mut gc = GarbageCollector::new();
        let t = Thread::new();
        let t_ref = gc.create(t);

        gc.reset_marks();

        // 无 caller — mark_children 不应 panic
        unsafe {
            let t_ptr = t_ref.as_ptr();
            (*t_ptr).mark_children(&mut gc);
        }
    }

    // ── GC 回收测试 ───────────────────────────────────────────────

    #[test]
    fn test_thread_swept_when_unreachable() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        gc.create(Thread::new());
        assert_eq!(gc.object_count(), 1);

        gc.mark();
        let collected = gc.sweep(&mut pool);
        assert_eq!(collected, 1);
        assert_eq!(gc.object_count(), 0);
    }

    #[test]
    fn test_thread_kept_when_root() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        gc.create_root(Thread::new());
        assert_eq!(gc.object_count(), 1);

        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 1);
    }

    #[test]
    fn test_thread_caller_chain_gc() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();

        let caller = gc.create(Thread::new());
        let mut callee = Thread::new();
        callee.set_caller(Some(caller));
        gc.create_root(callee);

        assert_eq!(gc.object_count(), 2);

        // Root callee → marks caller → both survive
        let collected = gc.collect(&mut pool);
        assert_eq!(collected, 0);
        assert_eq!(gc.object_count(), 2);
    }

    // ── get_size ──────────────────────────────────────────────────

    #[test]
    fn test_thread_get_size() {
        let t = Thread::new();
        let size = t.get_size();
        assert!(size >= std::mem::size_of::<Thread>());
    }

    // ── Debug 输出 ────────────────────────────────────────────────

    #[test]
    fn test_thread_debug() {
        let t = Thread::new();
        let debug_str = format!("{:?}", t);
        assert!(debug_str.contains("Suspended"), "Got '{}'", debug_str);
        assert!(debug_str.contains("Thread"));
    }

    #[test]
    fn test_thread_debug_with_caller() {
        let mut gc = GarbageCollector::new();
        let caller = gc.create(Thread::new());
        let mut t = Thread::new();
        t.set_caller(Some(caller));

        let debug_str = format!("{:?}", t);
        assert!(debug_str.contains("Thread"));
    }

    // ── CoroutineStatus 判别值 ────────────────────────────────────

    #[test]
    fn test_coroutine_status_discriminants() {
        assert_eq!(CoroutineStatus::Suspended as u8, 0);
        assert_eq!(CoroutineStatus::Running as u8, 1);
        assert_eq!(CoroutineStatus::Normal as u8, 2);
        assert_eq!(CoroutineStatus::Dead as u8, 3);
    }
}
