//! Lua 线程状态 (LuaState)
//!
//! 每个 LuaState 代表一个独立的协程执行环境，包含值栈、调用栈、
//! 全局表引用和线程状态。
//!

use std::collections::HashMap;

use lua_core::gc::collector::{GarbageCollector, GcRefValidationError};
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::state_handle::StateHandle;
use lua_core::string_pool::StringPool;
use lua_core::table::Table;
use lua_core::thread::Thread;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use super::call_info::CallInfo;
use super::stack::Stack;
use crate::native::{
    DeferredNativeCall, DeferredVmContinuation, NativeRequestId, NativeRequestPublishError,
    ResumeEnvelope, ResumeRequest,
};
use crate::runtime::StateArena;
use thiserror::Error;

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

/// State-local work and debt observed during deterministic Runtime shutdown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LuaStateShutdownReport {
    pub closed_open_upvalues: usize,
    pub rejected_open_upvalue_edges: usize,
    pub open_upvalue_owner_mismatches: usize,
    pub open_upvalue_stack_values_missing: usize,
}

impl LuaStateShutdownReport {
    pub fn merge(&mut self, other: Self) {
        self.closed_open_upvalues += other.closed_open_upvalues;
        self.rejected_open_upvalue_edges += other.rejected_open_upvalue_edges;
        self.open_upvalue_owner_mismatches += other.open_upvalue_owner_mismatches;
        self.open_upvalue_stack_values_missing += other.open_upvalue_stack_values_missing;
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum OpenUpvalueError {
    #[error("open Upvalue owner state is detached from its Runtime")]
    DetachedState,
    #[error("open Upvalue reference is invalid: {0}")]
    InvalidReference(#[from] GcRefValidationError),
    #[error("closed Upvalue remained in a state's open set")]
    ClosedInOpenSet,
    #[error("open Upvalue owner mismatch: expected {expected:?}, found {actual:?}")]
    OwnerMismatch {
        expected: StateHandle,
        actual: StateHandle,
    },
    #[error("open Upvalue stack slot {index} is outside the initialized stack")]
    StackValueMissing { index: usize },
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
    /// Runtime registry table. The Runtime owns the canonical root; reachable
    /// states retain the same handle for VM and library access.
    pub registry: Option<GcRef<Table>>,
    /// Coroutine object that owns this state, if any.
    pub current_thread: Option<GcRef<Thread>>,
    /// Optional metatable for nil values, configured through debug.setmetatable.
    pub nil_metatable: Option<GcRef<Table>>,
    /// Optional metatable for boolean values, configured through debug.setmetatable.
    pub boolean_metatable: Option<GcRef<Table>>,
    /// Optional metatable for number values, configured through debug.setmetatable.
    pub number_metatable: Option<GcRef<Table>>,
    /// This state's runtime-scoped generational identity.
    pub(crate) state_handle: Option<StateHandle>,
    /// Stable transitional pointer to the Runtime-owned StateArena.
    pub(crate) state_arena: Option<std::ptr::NonNull<StateArena>>,
    /// Whether Runtime opened publication for the current state turn.
    native_request_scope: bool,
    /// Next per-state request identifier. Zero is permanently reserved.
    next_native_request_id: u64,
    /// Request published by the current sealed Runtime-native operation.
    pending_native_request: Option<ResumeRequest>,
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
    pub debug_hook_skip_proto: Option<GcRef<Proto>>,
    /// Caller source line to skip immediately after sethook.
    pub debug_hook_skip_line: i32,
    /// Cache for debug.getinfo(..., "n") call-site name lookups.
    pub debug_name_cache: HashMap<(usize, usize, usize), (Option<String>, String)>,
    /// Open Upvalues owned by this state, sorted by descending stack index.
    ///
    /// The container owns list topology; Upvalue objects carry no intrusive
    /// successor or raw Stack pointer.
    pub open_upvalues: Vec<GcRef<Upvalue>>,
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
            registry: None,
            current_thread: None,
            nil_metatable: None,
            boolean_metatable: None,
            number_metatable: None,
            state_handle: None,
            state_arena: None,
            native_request_scope: false,
            next_native_request_id: 1,
            pending_native_request: None,
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
            open_upvalues: Vec::new(),
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

    /// Read one live Lua string only within the owner collector's validation
    /// scope. The callback cannot return a borrow tied to the managed object.
    pub fn with_string_bytes<R>(
        &self,
        string: GcRef<GcString>,
        read: impl for<'a> FnOnce(&'a [u8]) -> R,
    ) -> Result<R, GcRefValidationError> {
        let (gc, _) = super::vm_context::active_service_pointers(self)
            .ok_or(GcRefValidationError::CollectorUnavailable)?;
        // SAFETY: the dynamically scoped context is installed only while this
        // exact state and Heap are exclusively active.
        unsafe { gc.as_ref() }.with_string_bytes(string, read)
    }

    /// Copy one live Lua string after collector validation.
    pub fn copy_string_bytes(
        &self,
        string: GcRef<GcString>,
    ) -> Result<Vec<u8>, GcRefValidationError> {
        self.with_string_bytes(string, <[u8]>::to_vec)
    }

    /// Resolve an already-interned name without allocating or dereferencing
    /// candidate object memory before collector validation.
    pub fn find_interned_string(
        &self,
        bytes: &[u8],
    ) -> Result<Option<GcRef<GcString>>, GcRefValidationError> {
        let (gc, pool) = super::vm_context::active_service_pointers(self)
            .ok_or(GcRefValidationError::StringPoolUnavailable)?;
        // SAFETY: both pointers belong to the active context for this state.
        let Some(candidate) = (unsafe { pool.as_ref() }).find_bytes(bytes) else {
            return Ok(None);
        };
        // SAFETY: same dynamically scoped context; no object borrow escapes.
        unsafe { gc.as_ref() }.with_ref(candidate, |_| ())?;
        Ok(Some(candidate))
    }

    /// Return the active collector pointer for the dynamic native callback.
    ///
    /// The pointer is absent outside a Runtime/VM context and must never be
    /// stored past the current callback.
    #[doc(hidden)]
    pub fn active_gc_ptr(&self) -> Option<*mut GarbageCollector> {
        super::vm_context::active_service_pointers(self).map(|(collector, _)| collector.as_ptr())
    }

    /// Return the active canonical StringPool pointer for this state turn.
    ///
    /// The pointer is absent outside a Runtime/VM context and must never be
    /// stored past the current callback.
    #[doc(hidden)]
    pub fn active_string_pool_ptr(&self) -> Option<*mut StringPool> {
        super::vm_context::active_service_pointers(self).map(|(_, strings)| strings.as_ptr())
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
        self.call_stack[self.current_ci].reset();
        &mut self.call_stack[self.current_ci]
    }

    /// 弹出当前 CallInfo（函数返回后）
    pub fn pop_call_info(&mut self) {
        self.call_stack[self.current_ci].reset();
        if self.current_ci > 0 {
            self.current_ci -= 1;
        }
    }

    /// 清理并弹出所有高于 `target_ci` 的活动调用帧。
    ///
    /// 错误展开必须通过此入口，避免复用帧继续保留 Proto/varargs 根。
    pub fn unwind_call_info_to(&mut self, target_ci: usize) {
        while self.current_ci > target_ci {
            self.pop_call_info();
        }
    }

    /// 调用栈大小
    pub fn call_stack_size(&self) -> usize {
        self.current_ci + 1
    }

    // ── Upvalue 管理 ────────────────────────────────────────────

    pub(crate) fn find_or_create_upvalue(
        &mut self,
        stack_index: usize,
        gc: &mut GarbageCollector,
    ) -> Result<GcRef<Upvalue>, OpenUpvalueError> {
        let owner = self.state_handle.ok_or(OpenUpvalueError::DetachedState)?;
        let mut insertion_index = self.open_upvalues.len();
        for (position, &candidate) in self.open_upvalues.iter().enumerate() {
            let (candidate_owner, candidate_index) = gc
                .with_ref(candidate, Upvalue::open_location)?
                .ok_or(OpenUpvalueError::ClosedInOpenSet)?;
            if candidate_owner != owner {
                return Err(OpenUpvalueError::OwnerMismatch {
                    expected: owner,
                    actual: candidate_owner,
                });
            }
            if candidate_index == stack_index {
                return Ok(candidate);
            }
            if candidate_index < stack_index {
                insertion_index = position;
                break;
            }
        }

        gc.with_publication(|transaction| {
            let upvalue = transaction.alloc(Upvalue::new_open(owner, stack_index));
            // SAFETY: the callback installs the Upvalue in the owner state's
            // traced canonical open-Upvalue set before releasing its root.
            unsafe {
                transaction.publish_upvalue_reference(upvalue, |reference| {
                    self.open_upvalues.insert(insertion_index, reference);
                    reference
                })
            }
        })
        .map_err(OpenUpvalueError::InvalidReference)
    }

    pub(crate) fn close_upvalues(
        &mut self,
        level: usize,
        gc: &mut GarbageCollector,
    ) -> Result<(), OpenUpvalueError> {
        if self.open_upvalues.is_empty() {
            return Ok(());
        }
        let owner = self.state_handle.ok_or(OpenUpvalueError::DetachedState)?;
        while let Some(&uv_ref) = self.open_upvalues.first() {
            let (upvalue_owner, stack_index) = gc
                .with_ref(uv_ref, Upvalue::open_location)?
                .ok_or(OpenUpvalueError::ClosedInOpenSet)?;
            if upvalue_owner != owner {
                return Err(OpenUpvalueError::OwnerMismatch {
                    expected: owner,
                    actual: upvalue_owner,
                });
            }
            if stack_index < level {
                break;
            }

            let value = self
                .stack
                .at(stack_index)
                .cloned()
                .ok_or(OpenUpvalueError::StackValueMissing { index: stack_index })?;
            gc.with_mut(uv_ref, |upvalue| upvalue.close(value))?;
            self.open_upvalues.remove(0);
        }
        Ok(())
    }

    /// Close validated open Upvalues and detach every transitional Runtime
    /// backpointer before this state allocation is dropped.
    ///
    /// The caller must invoke this before the StateArena advances or retires
    /// this state's handle generation.
    pub(crate) fn prepare_for_runtime_shutdown(
        &mut self,
        gc: &mut GarbageCollector,
    ) -> LuaStateShutdownReport {
        let mut report = LuaStateShutdownReport::default();
        let owner = self.state_handle;

        for upvalue_ref in std::mem::take(&mut self.open_upvalues) {
            let location = match gc.with_ref(upvalue_ref, Upvalue::open_location) {
                Ok(Some(location)) => location,
                Ok(None) | Err(_) => {
                    report.rejected_open_upvalue_edges += 1;
                    continue;
                }
            };
            let (upvalue_owner, stack_index) = location;
            if owner != Some(upvalue_owner) {
                report.open_upvalue_owner_mismatches += 1;
                continue;
            }
            let Some(stack_value) = self.stack.at(stack_index).cloned() else {
                report.open_upvalue_stack_values_missing += 1;
                continue;
            };
            if gc
                .with_mut(upvalue_ref, |upvalue| upvalue.close(stack_value))
                .is_err()
            {
                report.rejected_open_upvalue_edges += 1;
                continue;
            }
            report.closed_open_upvalues += 1;
        }

        self.global_table = None;
        self.thread_env = None;
        self.chunk_env = None;
        self.registry = None;
        self.current_thread = None;
        self.nil_metatable = None;
        self.boolean_metatable = None;
        self.number_metatable = None;
        self.debug_hook = None;
        self.yielded_values.clear();
        self.last_error = None;
        self.debug_hook_skip_proto = None;
        self.state_handle = None;
        self.state_arena = None;
        self.native_request_scope = false;
        self.pending_native_request = None;

        report
    }

    // ── 线程状态 ────────────────────────────────────────────────

    pub fn get_status(&self) -> ThreadStatus {
        self.status
    }

    pub fn set_status(&mut self, s: ThreadStatus) {
        self.status = s;
    }

    pub(crate) fn with_native_request_scope<R>(
        &mut self,
        f: impl for<'scope> FnOnce(&'scope mut LuaState) -> R,
    ) -> Result<R, NativeRequestPublishError> {
        if self.native_request_scope || self.pending_native_request.is_some() {
            return Err(NativeRequestPublishError::MailboxOccupied);
        }
        self.native_request_scope = true;
        let guard = NativeRequestScopeGuard {
            state: std::ptr::NonNull::from(&mut *self),
        };
        let result = f(self);
        drop(guard);
        Ok(result)
    }

    pub(crate) fn publish_resume_request(
        &mut self,
        thread: GcRef<Thread>,
        target: StateHandle,
        args: Vec<Value>,
        envelope: ResumeEnvelope,
    ) -> Result<NativeRequestId, NativeRequestPublishError> {
        if !self.native_request_scope {
            return Err(NativeRequestPublishError::ScopeUnavailable);
        }
        if self.pending_native_request.is_some() {
            return Err(NativeRequestPublishError::MailboxOccupied);
        }
        let id = NativeRequestId::new(self.next_native_request_id);
        self.next_native_request_id = self
            .next_native_request_id
            .checked_add(1)
            .ok_or(NativeRequestPublishError::IdExhausted)?;
        self.pending_native_request = Some(ResumeRequest {
            id,
            thread,
            target,
            args,
            envelope,
            deferred: None,
        });
        Ok(id)
    }

    pub(crate) fn pending_native_request_id(&self) -> Option<NativeRequestId> {
        self.pending_native_request
            .as_ref()
            .map(|request| request.id)
    }

    pub(crate) fn seal_native_request(
        &mut self,
        id: NativeRequestId,
        deferred: DeferredNativeCall,
    ) -> bool {
        let Some(request) = self.pending_native_request.as_mut() else {
            return false;
        };
        if request.id != id || deferred.request_id != id || request.deferred.is_some() {
            return false;
        }
        request.deferred = Some(deferred);
        true
    }

    pub(crate) fn take_native_request(&mut self, id: NativeRequestId) -> Option<ResumeRequest> {
        if self
            .pending_native_request
            .as_ref()
            .is_some_and(|request| request.id == id && request.deferred.is_some())
        {
            self.pending_native_request.take()
        } else {
            None
        }
    }

    pub(crate) fn set_native_request_continuation(
        &mut self,
        id: NativeRequestId,
        continuation: DeferredVmContinuation,
    ) -> bool {
        let Some(request) = self.pending_native_request.as_mut() else {
            return false;
        };
        let Some(deferred) = request.deferred.as_mut() else {
            return false;
        };
        if request.id != id || deferred.request_id != id {
            return false;
        }
        deferred.continuation = continuation;
        true
    }

    pub(crate) fn retarget_native_request_to_protected_resume(
        &mut self,
        id: NativeRequestId,
    ) -> bool {
        let Some(request) = self.pending_native_request.as_mut() else {
            return false;
        };
        if request.id != id {
            return false;
        }
        request.envelope = match request.envelope {
            ResumeEnvelope::Resume => ResumeEnvelope::ProtectedResume,
            ResumeEnvelope::Wrap => ResumeEnvelope::ProtectedWrap,
            ResumeEnvelope::ProtectedResume | ResumeEnvelope::ProtectedWrap => return false,
        };
        request.deferred = None;
        true
    }
}

struct NativeRequestScopeGuard {
    state: std::ptr::NonNull<LuaState>,
}

impl Drop for NativeRequestScopeGuard {
    fn drop(&mut self) {
        // SAFETY: the guard is created from the uniquely borrowed LuaState and
        // drops before `with_native_request_scope` returns or unwinds.
        let state = unsafe { self.state.as_mut() };
        state.native_request_scope = false;
        if std::thread::panicking() {
            state.pending_native_request = None;
        }
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

    #[test]
    fn detached_state_cannot_publish_an_open_upvalue() {
        let mut gc = GarbageCollector::new();
        let mut state = LuaState::new();
        state.push_number(1.0);

        assert_eq!(
            state.find_or_create_upvalue(0, &mut gc),
            Err(OpenUpvalueError::DetachedState)
        );
    }

    #[test]
    fn popped_and_unwound_frames_release_proto_and_varargs() {
        let mut gc = GarbageCollector::new();
        let proto = gc.create(Proto::new());
        let mut ls = LuaState::new();

        let first = ls.push_call_info();
        first.proto = Some(proto);
        first.varargs.push(Value::Number(1.0));
        ls.pop_call_info();
        assert!(ls.call_stack[1].proto.is_none());
        assert!(ls.call_stack[1].varargs.is_empty());

        let reused = ls.push_call_info();
        assert!(reused.proto.is_none());
        assert!(reused.varargs.is_empty());
        reused.proto = Some(proto);
        reused.varargs.push(Value::Number(2.0));
        let second = ls.push_call_info();
        second.proto = Some(proto);
        second.varargs.push(Value::Number(3.0));

        ls.unwind_call_info_to(0);
        assert_eq!(ls.current_ci, 0);
        assert!(
            ls.call_stack[1..]
                .iter()
                .all(|ci| ci.proto.is_none() && ci.varargs.is_empty())
        );
    }
}
