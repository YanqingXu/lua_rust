//! Lua 虚拟机执行引擎
#![allow(clippy::collapsible_if, clippy::collapsible_match)]
//!
//! 基于寄存器的字节码解释器，实现全部 38 条 Lua 5.1 指令。
//!

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::function::{CFunction, Function, RuntimeNativeFunction};
use lua_core::gc::collector::{GarbageCollector, GcRefValidationError};
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::publication::{PublicationTxn, Rooted};
use lua_core::gc_string::GcString;
use lua_core::proto::{Proto, VARARG_NEEDSARG};
use lua_core::table::Table;
use lua_core::thread::{CoroutineStatus, Thread};
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;
use std::cmp::Ordering;
use std::ptr::NonNull;

use crate::native::{
    CallbackHelperKind, DebugHookContinuation, DebugHookNativeCall, DebugUpvalueOperation,
    DeferredNativeCall, DeferredVmContinuation, FullCollectionResult, GsubReplacement, GsubState,
    GsubStep, NativeRequestId, NativeRequestPublishError, PackageModuleOptionsState,
    ProtectedCallKind, ResumeEnvelope, ResumeResponse, TableForeachCursor, TableForeachState,
    TableSortPending, TableSortStage, TableSortState, UpvalueAccessOperation, UpvalueAccessRequest,
    VmMetamethodContinuation,
};
use crate::state::{LUA_MULTRET, LuaState, ThreadStatus, enter_vm_context};
use crate::trace::{
    begin_trace_callback_barrier, begin_trace_callback_barrier_for_frame,
    discard_instruction_diffs_above_frame, emit_call_trace, emit_coroutine_yield_trace,
    emit_instruction_snapshot, emit_return_trace, end_trace_callback_barrier,
    flush_instruction_diff, record_fused_jump_trace_suppression, record_trace_nil_range,
    record_trace_number_override, record_trace_upvalue_closes, record_trace_upvalue_open,
};

/// 执行结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecResult {
    Returned,
    Yielded,
}

/// Reason the VM returned control to its Runtime owner.
#[derive(Debug, Clone)]
pub enum VmExit {
    /// Ordinary Lua return or coroutine yield.
    Complete(ExecResult),
    /// A sealed native operation requested a state switch.
    NativeRequest(NativeRequestId),
    /// An opcode must access an open Upvalue owned by another state.
    UpvalueAccess(Box<UpvalueAccessRequest>),
    /// Managed allocation crossed the automatic-GC threshold at an
    /// instruction boundary.
    AutomaticGc,
}

/// Immutable callback metadata for one match selected by the stdlib `gsub`
/// pattern engine.
#[derive(Clone, Debug)]
pub struct GsubCallbackStep {
    pub prefix: Vec<u8>,
    pub matched: Vec<u8>,
    pub arguments: Vec<Value>,
    pub table_key: Value,
}

/// Callback-capable `gsub` replacement owned by the Runtime continuation.
#[derive(Clone, Debug)]
pub enum GsubCallbackReplacement {
    Function(Value),
    Table {
        table: GcRef<Table>,
        index_key: Value,
    },
}

/// Initial result of preparing callback-capable `gsub` work.
#[derive(Debug)]
pub enum GsubCallbackRequestOutcome {
    Suspended,
    Complete { output: Vec<u8>, count: usize },
}

enum VmMetamethodOutcome<T> {
    Ready(T),
    Suspended(NativeRequestId),
}

/// 最大嵌套调用深度
const MAX_CALLS: i32 = 512;
const MAX_STRING_LENGTH: usize = 64 * 1024 * 1024;

fn rooted_vm_bytes<'scope>(
    state: &LuaState,
    transaction: &mut PublicationTxn<'scope>,
    bytes: &[u8],
) -> Result<Rooted<'scope, GcString>, GcRefValidationError> {
    let pool = state
        .active_string_pool_ptr()
        .ok_or(GcRefValidationError::StringPoolUnavailable)?;
    // SAFETY: the pointer belongs to the dynamically scoped VM context for
    // this exact state turn.
    transaction.intern_bytes(unsafe { &mut *pool }, bytes)
}

/// Compile one byte-oriented Lua chunk and publish the resulting closure onto
/// the active state stack before releasing construction roots.
pub fn compile_chunk_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    source: &[u8],
    chunk_name: &[u8],
) -> Result<GcRef<Function>, String> {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser.parse().map_err(|err| {
        format!(
            "{}:{}: {}",
            chunk_name_for_diagnostic(chunk_name),
            err.line,
            err.message
        )
    })?;

    let environment = l.thread_env.or(l.global_table);
    let pool_ptr = l
        .active_string_pool_ptr()
        .ok_or_else(|| "chunk compilation unavailable without an active StringPool".to_string())?;
    gc.with_publication(|transaction| {
        // SAFETY: the pool belongs to this dynamically scoped Runtime turn.
        let generator =
            CodeGenerator::new_in_publication_with_pool(transaction, unsafe { &mut *pool_ptr });
        let proto = generator
            .generate_with_source_bytes(&chunk, chunk_name)
            .map_err(|err| format!("{}:{err}", chunk_name_for_diagnostic(chunk_name)))?;
        let proto = transaction.alloc(proto);
        let function = transaction
            .alloc_lua_function(&proto)
            .map_err(|err| format!("invalid chunk Function publication: {err}"))?;
        transaction
            .set_lua_function_environment(&function, environment)
            .map_err(|err| format!("invalid chunk environment publication: {err}"))?;

        // SAFETY: the callback installs the Function on the active Lua stack
        // before its object root is released.
        unsafe {
            transaction.publish_function_value(function, |value| {
                let Value::Function(function) = value.clone() else {
                    unreachable!("typed Function publication produced another Value kind");
                };
                l.push_value(value);
                function
            })
        }
        .map_err(|err| format!("invalid chunk stack publication: {err}"))
    })
}

/// Render an internal Lua source name as the chunk id used in diagnostics.
///
/// `@file` and `=literal` are storage conventions; neither marker is part of
/// the user-facing name. Other names are source snippets and use the fixed
/// C++ oracle's `[string "..."]` envelope.
pub fn chunk_name_for_diagnostic(chunk_name: &[u8]) -> String {
    if let Some(name) = chunk_name
        .strip_prefix(b"=")
        .or_else(|| chunk_name.strip_prefix(b"@"))
    {
        return diagnostic_bytes(name);
    }

    const SNIPPET_LIMIT: usize = 60;
    let mut snippet = String::new();
    let mut truncated = false;
    for &byte in chunk_name {
        if matches!(byte, b'\n' | b'\r') || snippet.len() >= SNIPPET_LIMIT {
            truncated = true;
            break;
        }
        if matches!(byte, b'"' | b'\\') {
            snippet.push('\\');
        }
        push_diagnostic_byte(&mut snippet, byte);
    }
    if truncated {
        snippet.push_str("...");
    }
    format!("[string \"{snippet}\"]")
}

fn diagnostic_bytes(bytes: &[u8]) -> String {
    let mut rendered = String::with_capacity(bytes.len());
    for &byte in bytes {
        push_diagnostic_byte(&mut rendered, byte);
    }
    rendered
}

fn push_diagnostic_byte(rendered: &mut String, byte: u8) {
    if byte.is_ascii_graphic() || byte == b' ' {
        rendered.push(char::from(byte));
    } else {
        use std::fmt::Write as _;
        let _ = write!(rendered, "\\x{byte:02x}");
    }
}

#[cfg(test)]
mod chunk_name_diagnostic_tests {
    use super::chunk_name_for_diagnostic;

    #[test]
    fn storage_markers_are_not_exposed_in_diagnostics() {
        assert_eq!(chunk_name_for_diagnostic(b"@dir/file.lua"), "dir/file.lua");
        assert_eq!(
            chunk_name_for_diagnostic(b"=(command line)"),
            "(command line)"
        );
    }

    #[test]
    fn source_names_use_the_lua_string_chunk_envelope() {
        assert_eq!(
            chunk_name_for_diagnostic(b"return \"value\""),
            "[string \"return \\\"value\\\"\"]"
        );
        assert_eq!(
            chunk_name_for_diagnostic(b"first line\nsecond line"),
            "[string \"first line...\"]"
        );
    }
}

pub(crate) fn definite_load_syntax_error(source: &[u8]) -> Option<String> {
    let first = source
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())?;
    if !matches!(
        first,
        b'*' | b'/' | b'%' | b'^' | b')' | b']' | b'}' | b',' | b'=' | b'<' | b'>'
    ) {
        return None;
    }

    let mut parser = Parser::from_bytes(source);
    match parser.parse() {
        Ok(_) => None,
        Err(err) if err.message.contains("<eof>") => None,
        Err(err) => Some(format!(
            "=(load):{}:{}: {}",
            err.line, err.column, err.message
        )),
    }
}

/// 虚拟机主执行循环
///
///
/// 参数：
/// - `l`: Lua 线程状态
/// - `proto`: 待执行的函数原型
/// - `gc`: 垃圾回收器（用于创建表、字符串等 GC 对象）
///
/// 局部变量（与 Lua C `luaV_execute` 对齐）：
/// - `ci` — 当前 CallInfo
/// - `cl` — 当前 Proto
/// - `base` — 栈基址指针（计算值 = &l.stack[ci.base]）
/// - `pc` — 程序计数器
pub fn execute_proto(
    l: &mut LuaState,
    proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    let entry_ci = l.current_ci;
    let result = execute_proto_inner(l, proto, gc, None);
    if result.is_err() && entry_ci == 0 && l.current_ci == 0 {
        let ci = l.current_call_info_mut();
        if ci.proto == Some(proto) {
            ci.proto = None;
            ci.varargs.clear();
        }
    }
    result
}

/// Resume the instruction whose pre-execution debug hook just completed.
/// The first iteration deliberately skips hook selection because count/line
/// bookkeeping for this exact PC was already performed before suspension.
pub(crate) fn resume_lua_instruction_after_debug_hook(
    l: &mut LuaState,
    proto: GcRef<Proto>,
    pc: usize,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    if l.current_call_info().proto != Some(proto) {
        return Err(RuntimeError::new(
            "debug-hook instruction continuation no longer matches its Lua frame",
        ));
    }
    execute_proto_inner(l, proto, gc, Some(pc))
}

/// Resume a freshly pushed Lua CALL frame after its call hook completes while
/// preserving the ordinary nested-call depth contract.
pub(crate) fn resume_lua_call_after_debug_hook(
    l: &mut LuaState,
    proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    let overflow = stack_overflow_error(l, gc, proto, 0);
    execute_counted_proto(l, proto, gc, overflow)
}

fn execute_proto_inner(
    l: &mut LuaState,
    proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
    forced_pc: Option<usize>,
) -> Result<VmExit, RuntimeError> {
    if l.nccalls > MAX_CALLS {
        return Err(RuntimeError::new(
            "VM: stack overflow (too many nested calls)",
        ));
    }
    let mut active_proto_ref = proto;
    let initial_max_stack = gc
        .with_ref(active_proto_ref, Proto::max_stack_size)
        .map_err(invalid_proto_error)?;
    let _nresults = l.current_call_info().nresults;
    let resume_from_yield = l.status == ThreadStatus::Yield;
    let mut skip_debug_hooks_once = forced_pc.is_some();
    let mut pc: usize = if let Some(pc) = forced_pc {
        l.status = ThreadStatus::Ok;
        let ci = l.current_call_info_mut();
        ci.savedpc = Some(pc);
        ci.proto = Some(active_proto_ref);
        pc
    } else if resume_from_yield {
        l.status = ThreadStatus::Ok;
        l.current_call_info_mut().proto = Some(active_proto_ref);
        l.current_call_info()
            .savedpc
            .map(|savedpc| savedpc + 1)
            .unwrap_or(0)
    } else {
        let ci = l.current_call_info_mut();
        ci.savedpc = Some(0); // start at PC 0
        ci.proto = Some(active_proto_ref);
        0
    };

    // Ensure stack has enough space for this function's registers.
    // Proto::max_stack_size() gives the number of register slots needed.
    let stack_needed = l.current_call_info().base + initial_max_stack as usize;
    if l.stack.size() < stack_needed {
        l.stack.set_top(stack_needed);
    }
    if l.current_call_info().top < stack_needed {
        l.current_call_info_mut().top = stack_needed;
    }

    // ANCHOR: lua_vm_dispatch
    // 主解释循环
    while let Some(inst) = proto_instruction(gc, active_proto_ref, pc)? {
        let op = opcode::get_opcode(inst);
        let base_idx = l.current_call_info().base;
        l.current_call_info_mut().savedpc = Some(pc);
        if skip_debug_hooks_once {
            skip_debug_hooks_once = false;
        } else if let Some(request_id) = run_debug_instruction_hooks(l, gc, active_proto_ref, pc)? {
            return Ok(VmExit::NativeRequest(request_id));
        }
        run_auto_weak_gc(l, gc);
        emit_instruction_snapshot(l, gc, active_proto_ref, pc, inst);

        match op {
            // ── 数据移动 (4) ─────────────────────────────────
            OpCode::MOVE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let src = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = src;
                }
            }

            OpCode::LOADK => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                let val = proto_constant(gc, active_proto_ref, bx)?;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = val;
                }
            }

            OpCode::LOADBOOL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Boolean(b != 0);
                }
                if c != 0 {
                    pc += 1; // skip next instruction
                }
            }

            OpCode::LOADNIL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                for i in a..=b {
                    if let Some(dst) = l.stack.at_mut(base_idx + i) {
                        *dst = Value::Nil;
                    }
                }
            }
            // ANCHOR_END: lua_vm_dispatch

            // ── 上值 / 全局 (3) ─────────────────────────────
            OpCode::GETUPVAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                // R(A) := UpValue[B]
                match get_upvalue(l, gc, b)? {
                    UpvalueRead::Ready(value) => {
                        if let Some(dst) = l.stack.at_mut(base_idx + a) {
                            *dst = value;
                        }
                    }
                    UpvalueRead::Remote {
                        upvalue,
                        owner,
                        stack_index,
                    } => {
                        let requester = l.state_handle().ok_or_else(|| {
                            RuntimeError::new(
                                "VM: open Upvalue requester is detached from its Runtime",
                            )
                        })?;
                        l.status = ThreadStatus::Yield;
                        return Ok(VmExit::UpvalueAccess(Box::new(UpvalueAccessRequest {
                            requester,
                            upvalue,
                            owner,
                            stack_index,
                            operation: UpvalueAccessOperation::Read {
                                destination: base_idx + a,
                            },
                        })));
                    }
                }
            }

            OpCode::GETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // R(A) := _G[K(Bx)]
                let name = proto_constant(gc, active_proto_ref, bx)?;
                if let VmMetamethodOutcome::Suspended(request_id) =
                    get_global_into(l, gc, &name, base_idx + a, active_proto_ref, pc)?
                {
                    return Ok(VmExit::NativeRequest(request_id));
                }
            }

            OpCode::GETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let table = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                if should_error_index(l, gc, &table) {
                    return Err(runtime_error_at(
                        gc,
                        active_proto_ref,
                        pc,
                        index_error_message(gc, active_proto_ref, pc, b, &table),
                    ));
                }
                if let VmMetamethodOutcome::Suspended(request_id) = get_table_into(
                    l,
                    gc,
                    &table,
                    &key,
                    base_idx + a,
                    VmMetamethodContinuation::StoreResult {
                        proto: active_proto_ref,
                        pc,
                        destination: base_idx + a,
                    },
                )? {
                    return Ok(VmExit::NativeRequest(request_id));
                }
            }

            // ── 变量赋值 (3) ─────────────────────────────────
            OpCode::SETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // _G[K(Bx)] := R(A)
                let name = proto_constant(gc, active_proto_ref, bx)?;
                let val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                if let VmMetamethodOutcome::Suspended(request_id) =
                    set_global(l, gc, &name, &val, active_proto_ref, pc)?
                {
                    return Ok(VmExit::NativeRequest(request_id));
                }
            }

            OpCode::SETUPVAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                // UpValue[B] := R(A)
                let val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                if let Some((upvalue, owner, stack_index)) = set_upvalue(l, gc, b, &val)? {
                    let requester = l.state_handle().ok_or_else(|| {
                        RuntimeError::new("VM: open Upvalue requester is detached from its Runtime")
                    })?;
                    l.status = ThreadStatus::Yield;
                    return Ok(VmExit::UpvalueAccess(Box::new(UpvalueAccessRequest {
                        requester,
                        upvalue,
                        owner,
                        stack_index,
                        operation: UpvalueAccessOperation::Write { value: val },
                    })));
                }
            }

            OpCode::SETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let key = get_rk(l, gc, active_proto_ref, base_idx, b)?;
                let value = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                let table_val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                if should_error_newindex(l, gc, &table_val) {
                    return Err(runtime_error_at(
                        gc,
                        active_proto_ref,
                        pc,
                        index_error_message(gc, active_proto_ref, pc, a, &table_val),
                    ));
                }
                if let VmMetamethodOutcome::Suspended(request_id) = set_table_value(
                    l,
                    gc,
                    &table_val,
                    &key,
                    &value,
                    VmMetamethodContinuation::CompleteSet {
                        proto: active_proto_ref,
                        pc,
                    },
                )? {
                    return Ok(VmExit::NativeRequest(request_id));
                }
            }

            // ── 表操作 (2) ───────────────────────────────────
            OpCode::NEWTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                // Create table with b (array) + c (hash) capacity hints
                gc.with_publication(|transaction| {
                    let table = transaction.alloc(Table::with_capacity(b as usize, c as usize));
                    // SAFETY: the callback installs the Table in an active VM
                    // register before releasing its temporary root.
                    unsafe {
                        transaction.publish_table_value(table, |value| {
                            if let Some(dst) = l.stack.at_mut(base_idx + a) {
                                *dst = value;
                            }
                        })
                    }
                })
                .map_err(|error| {
                    RuntimeError::new(format!("VM: could not publish new Table: {error}"))
                })?;
            }

            OpCode::SELF => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let obj = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                if should_error_index(l, gc, &obj) {
                    return Err(runtime_error_at(
                        gc,
                        active_proto_ref,
                        pc,
                        index_error_message(gc, active_proto_ref, pc, b, &obj),
                    ));
                }
                // R(A+1) = R(B)
                if let Some(dst) = l.stack.at_mut(base_idx + a + 1) {
                    *dst = obj.clone();
                }
                // R(A) = R(B)[RK(C)]
                if let VmMetamethodOutcome::Suspended(request_id) = get_table_into(
                    l,
                    gc,
                    &obj,
                    &key,
                    base_idx + a,
                    VmMetamethodContinuation::StoreSelfResult {
                        proto: active_proto_ref,
                        pc,
                        destination: base_idx + a,
                        self_destination: base_idx + a + 1,
                        object: obj.clone(),
                    },
                )? {
                    return Ok(VmExit::NativeRequest(request_id));
                }
            }

            // ── 算术运算 (6) ─────────────────────────────────
            OpCode::ADD | OpCode::SUB | OpCode::MUL | OpCode::DIV | OpCode::MOD | OpCode::POW => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, gc, active_proto_ref, base_idx, b)?;
                let rhs = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                match exec_arith_into(
                    l,
                    gc,
                    op,
                    &lhs,
                    &rhs,
                    base_idx + a,
                    VmMetamethodContinuation::StoreResult {
                        proto: active_proto_ref,
                        pc,
                        destination: base_idx + a,
                    },
                ) {
                    Ok(VmMetamethodOutcome::Ready(())) => {}
                    Ok(VmMetamethodOutcome::Suspended(request_id)) => {
                        return Ok(VmExit::NativeRequest(request_id));
                    }
                    Err(err)
                        if err.message == "attempt to perform arithmetic on a non-number value" =>
                    {
                        return Err(RuntimeError::new(arith_error_message(
                            gc,
                            active_proto_ref,
                            pc,
                            (b, &lhs),
                            (c, &rhs),
                        )));
                    }
                    Err(err) => return Err(err),
                }
            }

            // ── 一元运算 (3) ─────────────────────────────────
            OpCode::UNM => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let val = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                match exec_unm_into(
                    l,
                    gc,
                    &val,
                    base_idx + a,
                    VmMetamethodContinuation::StoreResult {
                        proto: active_proto_ref,
                        pc,
                        destination: base_idx + a,
                    },
                ) {
                    Ok(VmMetamethodOutcome::Ready(())) => {}
                    Ok(VmMetamethodOutcome::Suspended(request_id)) => {
                        return Ok(VmExit::NativeRequest(request_id));
                    }
                    Err(err)
                        if err.message == "attempt to perform arithmetic on a non-number value" =>
                    {
                        return Err(RuntimeError::new(unm_error_message(
                            gc,
                            active_proto_ref,
                            pc,
                            b,
                            &val,
                        )));
                    }
                    Err(err) => return Err(err),
                }
            }

            OpCode::NOT => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let val = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let result = Value::Boolean(val.is_false());
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
            }

            OpCode::LEN => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let val = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let destination = base_idx + a;
                match exec_len_into(
                    l,
                    gc,
                    &val,
                    destination,
                    VmMetamethodContinuation::StoreResult {
                        proto: active_proto_ref,
                        pc,
                        destination,
                    },
                )? {
                    VmMetamethodOutcome::Ready(()) => {}
                    VmMetamethodOutcome::Suspended(request_id) => {
                        return Ok(VmExit::NativeRequest(request_id));
                    }
                }
            }

            // ── 字符串连接 ────────────────────────────────────
            OpCode::CONCAT => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst) as usize;
                let stack_limit = base_idx + proto_max_stack_size(gc, active_proto_ref)?;
                let destination = base_idx + a;
                let initial = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                if let Some(dst) = l.stack.at_mut(destination) {
                    *dst = initial;
                }
                for i in (b + 1)..=c {
                    let rhs = l.stack.at(base_idx + i).cloned().unwrap_or(Value::Nil);
                    let lhs = l.stack.at(destination).cloned().unwrap_or(Value::Nil);
                    match exec_concat_into(
                        l,
                        gc,
                        &lhs,
                        &rhs,
                        destination,
                        VmMetamethodContinuation::Concat {
                            proto: active_proto_ref,
                            pc,
                            destination,
                            next_slot: base_idx + i + 1,
                            end_slot: base_idx + c,
                            stack_limit,
                        },
                    )? {
                        VmMetamethodOutcome::Ready(()) => {}
                        VmMetamethodOutcome::Suspended(request_id) => {
                            return Ok(VmExit::NativeRequest(request_id));
                        }
                    }
                }
            }

            // ── 控制流 (6) ───────────────────────────────────
            OpCode::JMP => {
                let sbx = opcode::get_arg_sbx(inst);
                // +1 compensates for Lua C's pre-increment fetch (pc++ before switch)
                pc = ((pc as i32) + sbx + 1) as usize;
                continue; // skip pc += 1 at end of loop
            }

            OpCode::EQ => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, gc, active_proto_ref, base_idx, b)?;
                let rhs = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                let equal = match exec_eq(
                    l,
                    gc,
                    &lhs,
                    &rhs,
                    VmMetamethodContinuation::Compare {
                        proto: active_proto_ref,
                        pc,
                        expected: a != 0,
                        invert_result: false,
                    },
                )? {
                    VmMetamethodOutcome::Ready(equal) => equal,
                    VmMetamethodOutcome::Suspended(request_id) => {
                        return Ok(VmExit::NativeRequest(request_id));
                    }
                };
                // Lua 5.1: skip when (equal as i32) != A
                if (equal && a == 0) || (!equal && a != 0) {
                    pc += 1; // skip next
                }
            }

            OpCode::LT => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, gc, active_proto_ref, base_idx, b)?;
                let rhs = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                let less = match exec_lt(
                    l,
                    gc,
                    &lhs,
                    &rhs,
                    VmMetamethodContinuation::Compare {
                        proto: active_proto_ref,
                        pc,
                        expected: a != 0,
                        invert_result: false,
                    },
                )? {
                    VmMetamethodOutcome::Ready(less) => less,
                    VmMetamethodOutcome::Suspended(request_id) => {
                        return Ok(VmExit::NativeRequest(request_id));
                    }
                };
                if (less && a == 0) || (!less && a != 0) {
                    pc += 1;
                }
            }

            OpCode::LE => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, gc, active_proto_ref, base_idx, b)?;
                let rhs = get_rk(l, gc, active_proto_ref, base_idx, c)?;
                let le = match exec_le(
                    l,
                    gc,
                    &lhs,
                    &rhs,
                    VmMetamethodContinuation::Compare {
                        proto: active_proto_ref,
                        pc,
                        expected: a != 0,
                        invert_result: false,
                    },
                )? {
                    VmMetamethodOutcome::Ready(le) => le,
                    VmMetamethodOutcome::Suspended(request_id) => {
                        return Ok(VmExit::NativeRequest(request_id));
                    }
                };
                if (le && a == 0) || (!le && a != 0) {
                    pc += 1;
                }
            }

            OpCode::TEST => {
                let a = opcode::get_arg_a(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let truthy = !val.is_false();
                if truthy == (c != 0)
                    && let Some(jump) = proto_instruction(gc, active_proto_ref, pc + 1)?
                    && opcode::get_opcode(jump) == OpCode::JMP
                {
                    record_fused_jump_trace_suppression(l, active_proto_ref, pc + 1);
                }
                // Lua 5.1: skip when (truthy as i32) != C
                if (truthy && c == 0) || (!truthy && c != 0) {
                    pc += 1;
                }
            }

            OpCode::TESTSET => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let val = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let truthy = !val.is_false();
                if (truthy && c == 0) || (!truthy && c != 0) {
                    pc += 1;
                } else {
                    // R(A) = R(B)
                    if let Some(src) = l.stack.at(base_idx + b).cloned() {
                        if let Some(dst) = l.stack.at_mut(base_idx + a) {
                            *dst = src;
                        }
                    }
                    if let Some(jump) = proto_instruction(gc, active_proto_ref, pc + 1)?
                        && opcode::get_opcode(jump) == OpCode::JMP
                    {
                        record_fused_jump_trace_suppression(l, active_proto_ref, pc + 1);
                    }
                }
            }

            // ── 函数调用 (3) ─────────────────────────────────
            OpCode::CALL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let func_pos = base_idx + a;
                let nargs = if b == 0 {
                    l.top.saturating_sub(func_pos + 1) as i32
                } else {
                    b - 1
                };
                let nresults = if c == 0 { LUA_MULTRET } else { c - 1 };
                let func = l.stack.at(func_pos).cloned().unwrap_or(Value::Nil);
                emit_call_trace(l, gc, active_proto_ref, pc, &func);

                if let Value::Function(func_ref) = func {
                    let (callee_proto, callable) = inspect_function(gc, func_ref)?;
                    if let Some(callee_proto_ref) = callee_proto {
                        // Setup new call frame
                        let new_base = base_idx + a + 1;
                        let (varargs, max_stack_size) =
                            prepare_lua_varargs(l, gc, callee_proto_ref, new_base, nargs)?;
                        let saved_ci = l.current_ci;
                        let ci = l.push_call_info();
                        ci.func = base_idx + a;
                        ci.base = new_base;
                        ci.top = new_base + max_stack_size;
                        ci.nresults = nresults;
                        ci.nargs = nargs;
                        ci.varargs = varargs;
                        ci.savedpc = None;
                        ci.proto = Some(callee_proto_ref);
                        ci.tailcalls = 0;

                        match publish_debug_hook_event(
                            l,
                            gc,
                            "call",
                            None,
                            DebugHookContinuation::LuaCall {
                                proto: callee_proto_ref,
                            },
                        ) {
                            Ok(Some(request_id)) => {
                                return Ok(VmExit::NativeRequest(request_id));
                            }
                            Ok(None) => {}
                            Err(error) => {
                                if l.current_thread.is_none() {
                                    unwind_lua_frames_to(l, gc, saved_ci)?;
                                }
                                return Err(error);
                            }
                        }

                        // Recursively execute the called function
                        match execute_nested_proto_at(l, active_proto_ref, pc, callee_proto_ref, gc)
                        {
                            Ok(VmExit::Complete(ExecResult::Returned)) => {
                                // Results already placed by RETURN handler
                            }
                            Ok(VmExit::Complete(ExecResult::Yielded)) => {
                                return Ok(VmExit::Complete(ExecResult::Yielded));
                            }
                            Ok(VmExit::NativeRequest(id)) => {
                                return Ok(VmExit::NativeRequest(id));
                            }
                            Ok(VmExit::UpvalueAccess(request)) => {
                                return Ok(VmExit::UpvalueAccess(request));
                            }
                            Ok(VmExit::AutomaticGc) => {
                                return Ok(VmExit::AutomaticGc);
                            }
                            Err(e) => {
                                // A failed coroutine keeps its Lua frames intact so
                                // debug.traceback(thread) can inspect the complete
                                // failure stack and any still-open Upvalues keep
                                // reading their owning registers. Main-state calls
                                // retain the eager unwind used by top-level errors;
                                // protected Runtime callbacks unwind explicitly at
                                // their stop_ci boundary.
                                if l.current_thread.is_none() {
                                    unwind_lua_frames_to(l, gc, saved_ci)?;
                                }
                                return Err(e);
                            }
                        }

                        if l.current_ci != saved_ci {
                            unwind_lua_frames_to(l, gc, saved_ci)?;
                            return Err(RuntimeError::new("VM: call frame imbalance"));
                        }
                        pc += 1;
                        continue;
                    } else if let Some(callable) = callable {
                        let func_pos = base_idx + a;
                        let actual_nargs = if b == 0 {
                            l.top.saturating_sub(func_pos + 1)
                        } else {
                            nargs as usize
                        };
                        let wanted_results = if c == 0 {
                            None
                        } else {
                            Some(nresults as usize)
                        };
                        match call_native_function(
                            l,
                            gc,
                            func_pos,
                            actual_nargs,
                            wanted_results,
                            callable,
                        ) {
                            Ok(VmExit::Complete(ExecResult::Yielded)) => {
                                return Ok(VmExit::Complete(ExecResult::Yielded));
                            }
                            Ok(VmExit::Complete(ExecResult::Returned)) => {}
                            Ok(VmExit::NativeRequest(id)) => {
                                return Ok(VmExit::NativeRequest(id));
                            }
                            Ok(VmExit::UpvalueAccess(request)) => {
                                return Ok(VmExit::UpvalueAccess(request));
                            }
                            Ok(VmExit::AutomaticGc) => {
                                return Ok(VmExit::AutomaticGc);
                            }
                            Err(err) if err.message.starts_with("bad argument") => {
                                return Err(runtime_error_at(
                                    gc,
                                    active_proto_ref,
                                    pc,
                                    err.message,
                                ));
                            }
                            Err(err) => return Err(err),
                        }
                    }
                } else if let Some(metamethod) = find_metamethod(l, gc, &func, &func, "__call") {
                    let actual_nargs = nargs.max(0) as usize;
                    ensure_stack_slot(l, func_pos + actual_nargs + 1);
                    for i in (0..=actual_nargs).rev() {
                        let src = l.stack.at(func_pos + i).cloned().unwrap_or(Value::Nil);
                        if let Some(dst) = l.stack.at_mut(func_pos + i + 1) {
                            *dst = src;
                        }
                    }
                    if let Some(dst) = l.stack.at_mut(func_pos) {
                        *dst = metamethod;
                    }
                    let wanted_results = if c == 0 {
                        None
                    } else {
                        Some(nresults as usize)
                    };
                    call_value_at_stack(l, gc, func_pos, actual_nargs + 1, wanted_results)?;
                } else {
                    let type_desc = match &func {
                        Value::Nil => "nil",
                        Value::Boolean(_) => "boolean",
                        Value::Number(_) => "number",
                        Value::String(_) => "string",
                        _ => "userdata",
                    };
                    let detail = describe_register_at(gc, active_proto_ref, pc, a, 8)
                        .map(|name| format!("{} (a {} value)", name, type_desc))
                        .unwrap_or_else(|| format!("a {} value", type_desc));
                    return Err(runtime_error_at(
                        gc,
                        active_proto_ref,
                        pc,
                        format!("attempt to call {detail}"),
                    ));
                }
            }

            OpCode::TAILCALL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let func_pos = base_idx + a;
                let nargs = if b == 0 {
                    l.top.saturating_sub(func_pos + 1) as i32
                } else {
                    b - 1
                };
                let mut actual_nargs = nargs.max(0) as usize;
                let func = l.stack.at(func_pos).cloned().unwrap_or(Value::Nil);
                if !matches!(func, Value::Function(_))
                    && let Some(metamethod) = find_metamethod(l, gc, &func, &func, "__call")
                {
                    ensure_stack_slot(l, func_pos + actual_nargs + 1);
                    for i in (0..=actual_nargs).rev() {
                        let src = l.stack.at(func_pos + i).cloned().unwrap_or(Value::Nil);
                        if let Some(dst) = l.stack.at_mut(func_pos + i + 1) {
                            *dst = src;
                        }
                    }
                    if let Some(dst) = l.stack.at_mut(func_pos) {
                        *dst = metamethod;
                    }
                    actual_nargs += 1;
                }
                let func = l.stack.at(func_pos).cloned().unwrap_or(Value::Nil);

                if l.current_call_info().func == base_idx {
                    let ci = l.current_call_info().clone();
                    let callable = match &func {
                        Value::Function(function) => inspect_function(gc, *function)?.1,
                        _ => None,
                    };
                    close_state_upvalues(l, gc, ci.base)?;
                    if let Some(callable) = callable {
                        match call_native_function(l, gc, func_pos, actual_nargs, None, callable)? {
                            VmExit::Complete(ExecResult::Returned) => {}
                            VmExit::Complete(ExecResult::Yielded) => {
                                return Ok(VmExit::Complete(ExecResult::Yielded));
                            }
                            VmExit::NativeRequest(id) => {
                                if !l.set_native_request_continuation(
                                    id,
                                    DeferredVmContinuation::TailCall,
                                ) {
                                    return Err(RuntimeError::new(
                                        "VM: failed to retain native tail-call continuation",
                                    ));
                                }
                                return Ok(VmExit::NativeRequest(id));
                            }
                            VmExit::UpvalueAccess(request) => {
                                return Ok(VmExit::UpvalueAccess(request));
                            }
                            VmExit::AutomaticGc => return Ok(VmExit::AutomaticGc),
                        }
                    } else {
                        call_value_at_stack(l, gc, func_pos, actual_nargs, None)?;
                    }
                    pc += 1;
                    continue;
                }

                if let Value::Function(func_ref) = func {
                    let (tail_proto, callable) = inspect_function(gc, func_ref)?;
                    if let Some(tail_proto_ref) = tail_proto.filter(|proto| !proto.is_null()) {
                        let args: Vec<Value> = (0..actual_nargs)
                            .map(|i| l.stack.at(func_pos + 1 + i).cloned().unwrap_or(Value::Nil))
                            .collect();
                        let old_frame_top = l.current_call_info().top;
                        close_state_upvalues(l, gc, base_idx)?;
                        for (i, arg) in args.iter().enumerate().take(actual_nargs) {
                            if let Some(dst) = l.stack.at_mut(base_idx + i) {
                                *dst = arg.clone();
                            }
                        }

                        let tail_nargs = actual_nargs as i32;
                        let (varargs, max_stack_size) =
                            prepare_lua_varargs(l, gc, tail_proto_ref, base_idx, tail_nargs)?;
                        let new_top = base_idx + max_stack_size;
                        for slot in new_top..old_frame_top.min(l.stack.size()) {
                            if let Some(value) = l.stack.at_mut(slot) {
                                *value = Value::Nil;
                            }
                        }
                        let frame_func = l.current_call_info().func;
                        if let Some(slot) = l.stack.at_mut(frame_func) {
                            *slot = Value::Function(func_ref);
                        }
                        let ci = l.current_call_info_mut();
                        ci.base = base_idx;
                        ci.top = new_top;
                        ci.nargs = tail_nargs;
                        ci.varargs = varargs;
                        ci.savedpc = Some(0);
                        ci.proto = Some(tail_proto_ref);
                        ci.tailcalls += 1;

                        if l.stack.size() < new_top {
                            l.stack.set_top(new_top);
                        }
                        if let Some(request_id) = publish_debug_hook_event(
                            l,
                            gc,
                            "call",
                            None,
                            DebugHookContinuation::LuaCall {
                                proto: tail_proto_ref,
                            },
                        )? {
                            return Ok(VmExit::NativeRequest(request_id));
                        }
                        active_proto_ref = tail_proto_ref;
                        pc = 0;
                        continue;
                    } else if let Some(callable) = callable {
                        close_state_upvalues(l, gc, base_idx)?;
                        match call_native_function(l, gc, func_pos, actual_nargs, None, callable)? {
                            VmExit::Complete(ExecResult::Yielded) => {
                                return Ok(VmExit::Complete(ExecResult::Yielded));
                            }
                            VmExit::NativeRequest(id) => {
                                if !l.set_native_request_continuation(
                                    id,
                                    DeferredVmContinuation::TailCall,
                                ) {
                                    return Err(RuntimeError::new(
                                        "VM: failed to retain native tail-call continuation",
                                    ));
                                }
                                return Ok(VmExit::NativeRequest(id));
                            }
                            VmExit::UpvalueAccess(request) => {
                                return Ok(VmExit::UpvalueAccess(request));
                            }
                            VmExit::AutomaticGc => return Ok(VmExit::AutomaticGc),
                            VmExit::Complete(ExecResult::Returned) => {}
                        }
                        pc += 1;
                        continue;
                    }
                }
                return Err(RuntimeError::new("tail call: only Lua functions supported"));
            }

            OpCode::RETURN => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as i32;
                emit_return_trace(l, gc, active_proto_ref, pc);
                let ci = l.current_call_info().clone();
                let available = if b == 0 {
                    l.top.saturating_sub(base_idx + a)
                } else {
                    (b - 1).max(0) as usize
                };
                let wanted = if l.current_ci == 0 || ci.nresults == LUA_MULTRET {
                    available
                } else {
                    ci.nresults.max(0) as usize
                };
                let src_base = base_idx + a;
                let mut results = Vec::with_capacity(wanted);
                for i in 0..wanted {
                    let result = if i < available {
                        l.stack.at(src_base + i).cloned().unwrap_or(Value::Nil)
                    } else {
                        Value::Nil
                    };
                    results.push(result);
                }

                close_state_upvalues(l, gc, ci.base)?;
                if debug_hook_event_enabled(l, "return") {
                    let remaining_tail_returns = ci.tailcalls;
                    let request_id = publish_debug_hook_event(
                        l,
                        gc,
                        "return",
                        None,
                        DebugHookContinuation::LuaReturn {
                            call_info: ci,
                            results,
                            remaining_tail_returns,
                        },
                    )?
                    .ok_or_else(|| {
                        RuntimeError::new("enabled Lua return hook was not published")
                    })?;
                    return Ok(VmExit::NativeRequest(request_id));
                }

                let result = finish_lua_return_after_debug_hook(l, &ci, results);
                flush_instruction_diff(l, gc);
                return result;
            }

            // ── 循环 (3) ─────────────────────────────────────
            OpCode::FORLOOP => {
                let a = opcode::get_arg_a(inst) as usize;
                let sbx = opcode::get_arg_sbx(inst);
                // R(A) += R(A+2), check against R(A+1)
                let step = l
                    .stack
                    .at(base_idx + a + 2)
                    .cloned()
                    .unwrap_or(Value::Number(1.0));
                let limit = l
                    .stack
                    .at(base_idx + a + 1)
                    .cloned()
                    .unwrap_or(Value::Number(0.0));
                let idx_val = l
                    .stack
                    .at(base_idx + a)
                    .cloned()
                    .unwrap_or(Value::Number(0.0));
                let step_num = as_number(gc, &step);
                let limit_num = as_number(gc, &limit);
                let idx_num = as_number(gc, &idx_val) + step_num;
                let continues = (step_num > 0.0 && idx_num <= limit_num)
                    || (step_num < 0.0 && idx_num >= limit_num);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Number(idx_num);
                }
                if let Some(dst) = l.stack.at_mut(base_idx + a + 3) {
                    *dst = Value::Number(idx_num);
                }
                if !continues {
                    let last_accepted = as_number(gc, &idx_val);
                    record_trace_number_override(l, base_idx + a, last_accepted);
                    record_trace_number_override(l, base_idx + a + 3, last_accepted);
                }
                if continues {
                    // +1 compensates for Lua C's pre-increment fetch
                    pc = ((pc as i32) + sbx + 1) as usize;
                    continue;
                }
            }

            OpCode::FORPREP => {
                let a = opcode::get_arg_a(inst) as usize;
                let sbx = opcode::get_arg_sbx(inst);
                // R(A) -= R(A+2)
                let step = l
                    .stack
                    .at(base_idx + a + 2)
                    .cloned()
                    .unwrap_or(Value::Number(1.0));
                let init = l
                    .stack
                    .at(base_idx + a)
                    .cloned()
                    .unwrap_or(Value::Number(0.0));
                let limit = l.stack.at(base_idx + a + 1).cloned().unwrap_or(Value::Nil);
                let step_num = match to_arith_number(gc, &step) {
                    Some(step) => step,
                    None => {
                        return Err(runtime_error_at(
                            gc,
                            active_proto_ref,
                            pc,
                            "'for' step must be a number",
                        ));
                    }
                };
                if to_arith_number(gc, &limit).is_none() {
                    return Err(runtime_error_at(
                        gc,
                        active_proto_ref,
                        pc,
                        "'for' limit must be a number",
                    ));
                }
                let init_num = match to_arith_number(gc, &init) {
                    Some(init) => init - step_num,
                    None => {
                        return Err(runtime_error_at(
                            gc,
                            active_proto_ref,
                            pc,
                            "'for' initial value must be a number",
                        ));
                    }
                };
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Number(init_num);
                }
                // +1 compensates for Lua C's pre-increment fetch
                pc = ((pc as i32) + sbx + 1) as usize;
                continue;
            }

            OpCode::TFORLOOP => {
                let a = opcode::get_arg_a(inst) as usize;
                let c = opcode::get_arg_c(inst) as usize;
                let func = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let state = l.stack.at(base_idx + a + 1).cloned().unwrap_or(Value::Nil);
                let control = l.stack.at(base_idx + a + 2).cloned().unwrap_or(Value::Nil);

                match &func {
                    Value::Function(func_ref) => {
                        let (iter_proto, callable) = inspect_function(gc, *func_ref)?;
                        if let Some(callable) = callable {
                            let call_pos = base_idx + a + 3;
                            ensure_stack_slot(l, call_pos + 2);
                            if let Some(dst) = l.stack.at_mut(call_pos) {
                                *dst = func.clone();
                            }
                            if let Some(dst) = l.stack.at_mut(call_pos + 1) {
                                *dst = state;
                            }
                            if let Some(dst) = l.stack.at_mut(call_pos + 2) {
                                *dst = control;
                            }
                            match call_native_function(l, gc, call_pos, 2, Some(c), callable)? {
                                VmExit::Complete(ExecResult::Yielded) => {
                                    return Ok(VmExit::Complete(ExecResult::Yielded));
                                }
                                VmExit::NativeRequest(id) => {
                                    if !l.set_native_request_continuation(
                                        id,
                                        DeferredVmContinuation::GenericFor {
                                            base: base_idx,
                                            register: a,
                                        },
                                    ) {
                                        return Err(RuntimeError::new(
                                            "VM: failed to attach generic-for continuation",
                                        ));
                                    }
                                    return Ok(VmExit::NativeRequest(id));
                                }
                                VmExit::UpvalueAccess(request) => {
                                    return Ok(VmExit::UpvalueAccess(request));
                                }
                                VmExit::AutomaticGc => return Ok(VmExit::AutomaticGc),
                                VmExit::Complete(ExecResult::Returned) => {}
                            }
                        } else if let Some(iter_proto_ref) = iter_proto {
                            let call_pos = base_idx + a + 3;
                            ensure_stack_slot(l, call_pos + 2);
                            if let Some(dst) = l.stack.at_mut(call_pos) {
                                *dst = func.clone();
                            }
                            if let Some(dst) = l.stack.at_mut(call_pos + 1) {
                                *dst = state;
                            }
                            if let Some(dst) = l.stack.at_mut(call_pos + 2) {
                                *dst = control;
                            }

                            let saved_ci = l.current_ci;
                            let (varargs, max_stack_size) =
                                prepare_lua_varargs(l, gc, iter_proto_ref, call_pos + 1, 2)?;
                            let ci = l.push_call_info();
                            ci.func = call_pos;
                            ci.base = call_pos + 1;
                            ci.top = ci.base + max_stack_size;
                            ci.nresults = c as i32;
                            ci.nargs = 2;
                            ci.varargs = varargs;
                            ci.savedpc = None;
                            ci.proto = Some(iter_proto_ref);
                            ci.tailcalls = 0;

                            match execute_nested_proto_at(
                                l,
                                active_proto_ref,
                                pc,
                                iter_proto_ref,
                                gc,
                            ) {
                                Ok(VmExit::Complete(ExecResult::Returned)) => {}
                                Ok(VmExit::Complete(ExecResult::Yielded)) => {
                                    return Ok(VmExit::Complete(ExecResult::Yielded));
                                }
                                Ok(VmExit::NativeRequest(id)) => {
                                    return Ok(VmExit::NativeRequest(id));
                                }
                                Ok(VmExit::UpvalueAccess(request)) => {
                                    return Ok(VmExit::UpvalueAccess(request));
                                }
                                Ok(VmExit::AutomaticGc) => {
                                    return Ok(VmExit::AutomaticGc);
                                }
                                Err(e) => {
                                    if l.current_thread.is_none() {
                                        unwind_lua_frames_to(l, gc, saved_ci)?;
                                    }
                                    return Err(e);
                                }
                            }

                            if l.current_ci != saved_ci {
                                unwind_lua_frames_to(l, gc, saved_ci)?;
                                return Err(RuntimeError::new(
                                    "VM: generic-for call frame imbalance",
                                ));
                            }
                        } else {
                            return Err(RuntimeError::new(
                                "generic for: iterator function has no callable body",
                            ));
                        }
                    }
                    _ => {
                        return Err(runtime_error_at(
                            gc,
                            active_proto_ref,
                            pc,
                            "generic for: iterator function expected",
                        ));
                    }
                }

                // If R(A+3) is nil, exit loop
                let first_result = l.stack.at(base_idx + a + 3).cloned().unwrap_or(Value::Nil);
                if first_result.is_nil() {
                    // skip next instruction (jump out of loop)
                    pc += 1;
                } else if let Some(dst) = l.stack.at_mut(base_idx + a + 2) {
                    *dst = first_result;
                }
            }

            // ── 表/栈/闭包/变参 (4) ─────────────────────────
            OpCode::SETLIST => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let mut c = opcode::get_arg_c(inst) as usize;
                if c == 0 {
                    pc += 1;
                    c = proto_instruction(gc, active_proto_ref, pc)?
                        .ok_or_else(|| RuntimeError::new("VM: SETLIST missing block argument"))?
                        as usize;
                    if c == 0 {
                        return Err(RuntimeError::new("VM: SETLIST invalid block argument"));
                    }
                }
                // R(A) is the table, R(A+1)..R(A+b) are values
                // c (or from instruction encoding block) is the block offset
                let count = if b > 0 {
                    b as usize
                } else {
                    l.top.saturating_sub(base_idx + a + 1)
                };
                if let Some(Value::Table(table_ref)) = l.stack.at(base_idx + a).cloned() {
                    let values = (1..=count)
                        .map(|i| {
                            (
                                ((c - 1) * 50) as i32 + i as i32,
                                l.stack.at(base_idx + a + i).cloned().unwrap_or(Value::Nil),
                            )
                        })
                        .collect::<Vec<_>>();
                    gc.with_mut(table_ref, |table| {
                        for (index, value) in &values {
                            table.set_array(*index, value);
                        }
                    })
                    .map_err(|error| {
                        RuntimeError::new(format!("VM: invalid SETLIST table: {error}"))
                    })?;
                }
            }

            OpCode::CLOSE => {
                let a = opcode::get_arg_a(inst) as usize;
                // Close upvalues at level A and above
                close_state_upvalues(l, gc, base_idx + a)?;
            }

            OpCode::CLOSURE => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // Create closure from sub-proto Bx
                let sub_proto_ref = gc
                    .with_ref(active_proto_ref, |proto| {
                        (bx < proto.sub_proto_count()).then(|| proto.sub_proto(bx))
                    })
                    .map_err(invalid_proto_error)?
                    .ok_or_else(|| RuntimeError::new("VM: CLOSURE proto index out of range"))?;
                if sub_proto_ref.is_null() {
                    return Err(RuntimeError::new("VM: CLOSURE proto index out of range"));
                }
                {
                    let environment = current_env(l, gc);
                    let child_upvalue_count = gc
                        .with_ref(sub_proto_ref, |proto| proto.num_upvalues() as usize)
                        .map_err(invalid_proto_error)?;
                    let mut next_pc = pc + 1;
                    let mut upvalues = Vec::with_capacity(child_upvalue_count);
                    for upvalue_index in 0..child_upvalue_count {
                        let pseudo =
                            proto_instruction(gc, active_proto_ref, next_pc)?.ok_or_else(|| {
                                RuntimeError::new("VM: CLOSURE missing upvalue pseudo instruction")
                            })?;
                        next_pc += 1;

                        let upvalue_ref = match opcode::get_opcode(pseudo) {
                            OpCode::MOVE => {
                                let b = opcode::get_arg_b(pseudo) as usize;
                                let open_count = l.open_upvalues.len();
                                let upvalue = l.find_or_create_upvalue(base_idx + b, gc).map_err(
                                    |error| {
                                        RuntimeError::new(format!(
                                            "VM: could not publish open Upvalue: {error}"
                                        ))
                                    },
                                )?;
                                if l.open_upvalues.len() > open_count {
                                    let name = gc
                                        .with_ref(sub_proto_ref, |proto| {
                                            proto.upvalue_name(upvalue_index)
                                        })
                                        .map_err(invalid_proto_error)?;
                                    record_trace_upvalue_open(
                                        l,
                                        gc,
                                        active_proto_ref,
                                        pc,
                                        base_idx + b,
                                        name,
                                    );
                                }
                                upvalue
                            }
                            OpCode::GETUPVAL => {
                                let b = opcode::get_arg_b(pseudo) as usize;
                                current_lua_upvalue(l, gc, b).ok_or_else(|| {
                                    RuntimeError::new("VM: CLOSURE invalid parent upvalue index")
                                })?
                            }
                            _ => {
                                return Err(RuntimeError::new(
                                    "VM: CLOSURE expects MOVE/GETUPVAL pseudo instruction",
                                ));
                            }
                        };
                        upvalues.push(upvalue_ref);
                    }

                    gc.with_publication(|transaction| {
                        let function =
                            transaction.alloc_lua_closure(sub_proto_ref, environment, &upvalues)?;
                        // SAFETY: the callback installs the closure in an
                        // active VM register before releasing its root.
                        unsafe {
                            transaction.publish_function_value(function, |value| {
                                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                                    *dst = value;
                                }
                            })
                        }
                    })
                    .map_err(|error| {
                        RuntimeError::new(format!("VM: could not publish closure: {error}"))
                    })?;
                    pc = next_pc - 1;
                }
            }

            OpCode::VARARG => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let varargs = l.current_call_info().varargs.clone();
                let available = varargs.len();
                let wanted = if b == 0 {
                    available
                } else {
                    (b - 1).max(0) as usize
                };

                if wanted > 0 {
                    ensure_stack_slot(l, base_idx + a + wanted - 1);
                }
                for i in 0..wanted {
                    let value = varargs.get(i).cloned().unwrap_or(Value::Nil);
                    if let Some(dst) = l.stack.at_mut(base_idx + a + i) {
                        *dst = value;
                    }
                }
                if b == 0 {
                    l.top = base_idx + a + wanted;
                }
            }
        }

        pc += 1;
        if gc.automatic_step_due() {
            // Diff snapshots own cloned Values that are not part of the GC root
            // graph. Complete the just-executed instruction before Runtime can
            // advance the collector at this instruction boundary.
            flush_instruction_diff(l, gc);
            // Some instructions consume following pseudo words or rewrite the
            // control PC (CLOSURE bindings, SETLIST extra words, LOADBOOL
            // skips, jumps). Persist the completed instruction's successor
            // before suspending for Runtime-owned GC; otherwise resume derives
            // the next PC from the stale opcode entry and can execute data as
            // bytecode.
            l.current_call_info_mut().savedpc = Some(pc.saturating_sub(1));
            l.status = ThreadStatus::Yield;
            return Ok(VmExit::AutomaticGc);
        }
    }

    flush_instruction_diff(l, gc);
    l.pop_call_info();
    Ok(VmExit::Complete(ExecResult::Returned))
}

fn run_debug_instruction_hooks(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
) -> Result<Option<NativeRequestId>, RuntimeError> {
    if l.debug_hook.is_none() || l.debug_hook_active {
        return Ok(None);
    }

    if l.debug_hook_count > 0 {
        l.debug_hook_countdown -= 1;
        if l.debug_hook_countdown <= 0 {
            l.debug_hook_countdown = l.debug_hook_count;
            return publish_debug_hook_event(
                l,
                gc,
                "count",
                None,
                DebugHookContinuation::InstructionAfterCount { proto, pc },
            );
        }
    }

    dispatch_debug_line_hook(l, gc, proto, pc)
}

pub(crate) fn dispatch_debug_line_hook(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
) -> Result<Option<NativeRequestId>, RuntimeError> {
    if l.debug_hook.is_none() || l.debug_hook_active {
        return Ok(None);
    }
    if l.debug_hook_mask.contains('l') {
        let line = gc
            .with_ref(proto, |active| active.line(pc))
            .map_err(invalid_proto_error)?;
        let should_skip_setup_line =
            l.debug_hook_skip_line == line && l.debug_hook_skip_proto == Some(proto);
        let repeated_line_from_jump = pc <= l.debug_hook_last_pc && line == l.debug_hook_last_line;
        if should_skip_setup_line {
            l.debug_hook_skip_proto = None;
            l.debug_hook_skip_line = -1;
            l.debug_hook_last_line = line;
            l.debug_hook_last_pc = pc;
        } else if line > 0 && (line != l.debug_hook_last_line || repeated_line_from_jump) {
            l.debug_hook_last_line = line;
            l.debug_hook_last_pc = pc;
            return publish_debug_hook_event(
                l,
                gc,
                "line",
                Some(line),
                DebugHookContinuation::ResumeAt { proto, pc },
            );
        } else {
            l.debug_hook_last_pc = pc;
        }
    }

    Ok(None)
}

fn run_auto_weak_gc(l: &mut LuaState, gc: &mut GarbageCollector) {
    if gc.is_automatic_stopped()
        || gc.incremental_phase() != lua_core::gc::collector::IncrementalPhase::Pause
        || l.debug_hook_active
        || !gc.has_seen_weak_table()
    {
        return;
    }

    l.auto_gc_countdown -= 1;
    if l.auto_gc_countdown > 0 {
        return;
    }
    l.auto_gc_countdown = 200;

    gc.reset_marks();
    mark_vm_roots_for_weak_cleanup(l, gc);
    gc.propagate_marks();
    gc.clear_registered_weak_tables();
}

fn mark_vm_roots_for_weak_cleanup(l: &LuaState, gc: &mut GarbageCollector) {
    if let Some(global_table) = l.global_table {
        gc.mark_value(&Value::Table(global_table));
    }
    if let Some(thread_env) = l.thread_env {
        gc.mark_value(&Value::Table(thread_env));
    }
    if let Some(chunk_env) = l.chunk_env {
        gc.mark_value(&Value::Table(chunk_env));
    }
    if let Some(thread) = l.current_thread {
        gc.mark_value(&Value::Thread(thread));
    }
    if let Some(hook) = &l.debug_hook {
        gc.mark_value(hook);
    }
    if let Some(skip_proto) = l.debug_hook_skip_proto {
        gc.mark_registered(skip_proto);
    }
    if let Some(error) = &l.last_error {
        gc.mark_value(error);
    }
    for value in &l.yielded_values {
        gc.mark_value(value);
    }

    mark_open_upvalues(l, gc);

    for ci in l.call_stack.iter().take(l.current_ci + 1) {
        if !(ci.func == ci.base && ci.proto.is_some())
            && let Some(value) = l.stack.at(ci.func)
        {
            gc.mark_value(value);
        }
        for value in &ci.varargs {
            gc.mark_value(value);
        }

        let Some(proto_ref) = frame_proto_for_gc(l, gc, ci) else {
            continue;
        };
        gc.mark_registered(proto_ref);
        let pc = ci.savedpc.unwrap_or(0) as i32;
        let local_slots = gc
            .with_ref(proto_ref, |proto| {
                (0..proto.loc_var_count())
                    .filter_map(|idx| {
                        let loc = proto.loc_var(idx);
                        (loc.startpc <= pc && pc < loc.endpc && loc.reg >= 0)
                            .then_some(ci.base + loc.reg as usize)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for stack_index in local_slots {
            if let Some(value) = l.stack.at(stack_index) {
                gc.mark_value(value);
            }
        }
    }
}

fn frame_proto_for_gc(
    l: &LuaState,
    gc: &GarbageCollector,
    ci: &crate::state::CallInfo,
) -> Option<GcRef<Proto>> {
    if let Some(proto) = ci.proto {
        return Some(proto);
    }
    let Value::Function(func_ref) = l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) else {
        return None;
    };
    gc.with_ref(func_ref, |func| {
        func.is_lua_function().then(|| func.proto()).flatten()
    })
    .ok()
    .flatten()
}

fn mark_open_upvalues(l: &LuaState, gc: &mut GarbageCollector) {
    for &upvalue_ref in &l.open_upvalues {
        let location = gc
            .with_ref(upvalue_ref, Upvalue::open_location)
            .ok()
            .flatten();
        gc.mark_registered(upvalue_ref);
        if let Some((owner, stack_index)) = location
            && l.state_handle() == Some(owner)
            && let Some(value) = l.stack.at(stack_index)
        {
            gc.mark_value(value);
        }
    }
}

fn close_state_upvalues(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    level: usize,
) -> Result<(), RuntimeError> {
    l.close_upvalues(level, gc)
        .map_err(|error| RuntimeError::new(format!("VM: could not close open Upvalue: {error}")))?;
    record_trace_upvalue_closes(l, gc, level);
    Ok(())
}

fn debug_hook_event_enabled(l: &LuaState, event: &str) -> bool {
    l.debug_hook.is_some()
        && !l.debug_hook_active
        && (!matches!(event, "call") || l.debug_hook_mask.contains('c'))
        && (!matches!(event, "return" | "tail return") || l.debug_hook_mask.contains('r'))
}

pub(crate) fn publish_debug_hook_event(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    event: &str,
    line: Option<i32>,
    continuation: DebugHookContinuation,
) -> Result<Option<NativeRequestId>, RuntimeError> {
    if !debug_hook_event_enabled(l, event) {
        return Ok(None);
    }
    let hook = l
        .debug_hook
        .clone()
        .expect("enabled debug hook has a function value");

    let saved_top = l.top;
    let stop_ci = l.current_ci;
    let call_pos = l.current_call_info().top.max(saved_top.saturating_add(1));
    let trace_barrier = match &continuation {
        DebugHookContinuation::ResumeAt { .. }
        | DebugHookContinuation::InstructionAfterCount { .. } => None,
        DebugHookContinuation::LuaReturn { .. } => begin_trace_callback_barrier(l),
        DebugHookContinuation::LuaCall { .. } => begin_trace_callback_barrier(l).or_else(|| {
            stop_ci
                .checked_sub(1)
                .and_then(|frame| begin_trace_callback_barrier_for_frame(l, frame))
        }),
        DebugHookContinuation::NativeCall { frame }
        | DebugHookContinuation::NativeReturn { frame, .. } => {
            begin_trace_callback_barrier_for_frame(l, frame.saved_ci)
        }
    };
    l.debug_hook_active = true;
    let publication = gc.with_publication(|transaction| {
        let event = rooted_vm_bytes(l, transaction, event.as_bytes())?;
        // SAFETY: the callback installs the event string in the rooted Runtime
        // request before its publication root is released.
        unsafe {
            transaction.publish_string_value(event, |event| {
                let line = line.map_or(Value::Nil, |line| Value::Number(line as f64));
                l.publish_callback_helper_request(
                    CallbackHelperKind::DebugHook {
                        stop_ci,
                        call_pos,
                        saved_top,
                        trace_barrier,
                        continuation: Box::new(continuation),
                    },
                    hook,
                    vec![event, line],
                )
            })
        }
    });
    match publication {
        Ok(Ok(request_id)) => Ok(Some(request_id)),
        Ok(Err(error)) => {
            l.debug_hook_active = false;
            end_trace_callback_barrier(l, trace_barrier);
            let message = match error {
                NativeRequestPublishError::ScopeUnavailable => {
                    "debug hook requires the Runtime dispatcher"
                }
                NativeRequestPublishError::MailboxOccupied => {
                    "debug hook found an occupied Runtime-native mailbox"
                }
                NativeRequestPublishError::IdExhausted => {
                    "debug hook Runtime-native request id exhausted"
                }
            };
            Err(RuntimeError::new(message))
        }
        Err(error) => {
            l.debug_hook_active = false;
            end_trace_callback_barrier(l, trace_barrier);
            Err(RuntimeError::new(format!(
                "invalid debug-hook event: {error}"
            )))
        }
    }
}

fn publish_vm_metamethod_call(
    l: &mut LuaState,
    function: Value,
    args: Vec<Value>,
    continuation: VmMetamethodContinuation,
) -> Result<NativeRequestId, RuntimeError> {
    let saved_top = l.top;
    let stop_ci = l.current_ci;
    let call_pos = l.current_call_info().top.max(saved_top.saturating_add(1));
    let trace_barrier = begin_trace_callback_barrier(l);
    let publication = l.publish_callback_helper_request(
        CallbackHelperKind::VmMetamethod {
            stop_ci,
            call_pos,
            saved_top,
            trace_barrier,
            continuation,
        },
        function,
        args,
    );
    publication.map_err(|error| {
        end_trace_callback_barrier(l, trace_barrier);
        let message = match error {
            NativeRequestPublishError::ScopeUnavailable => {
                "VM metamethod requires the Runtime dispatcher"
            }
            NativeRequestPublishError::MailboxOccupied => {
                "VM metamethod found an occupied Runtime-native mailbox"
            }
            NativeRequestPublishError::IdExhausted => {
                "VM metamethod Runtime-native request id exhausted"
            }
        };
        RuntimeError::new(message)
    })
}

pub(crate) fn finish_vm_metamethod_callback(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    continuation: VmMetamethodContinuation,
    results: Vec<Value>,
) -> Result<VmExit, RuntimeError> {
    let (proto, pc) = match &continuation {
        VmMetamethodContinuation::StoreResult { proto, pc, .. }
        | VmMetamethodContinuation::StoreSelfResult { proto, pc, .. }
        | VmMetamethodContinuation::CompleteSet { proto, pc }
        | VmMetamethodContinuation::Compare { proto, pc, .. }
        | VmMetamethodContinuation::Concat { proto, pc, .. } => (*proto, *pc),
    };
    if l.current_call_info().proto != Some(proto) || l.current_call_info().savedpc != Some(pc) {
        return Err(RuntimeError::new(
            "VM metamethod continuation no longer matches its opcode frame",
        ));
    }
    let result = results.into_iter().next().unwrap_or(Value::Nil);
    match continuation {
        VmMetamethodContinuation::StoreResult { destination, .. } => {
            ensure_stack_slot(l, destination);
            if let Some(slot) = l.stack.at_mut(destination) {
                *slot = result;
            }
        }
        VmMetamethodContinuation::StoreSelfResult {
            destination,
            self_destination,
            object,
            ..
        } => {
            ensure_stack_slot(l, destination.max(self_destination));
            if let Some(slot) = l.stack.at_mut(destination) {
                *slot = result;
            }
            if let Some(slot) = l.stack.at_mut(self_destination) {
                *slot = object;
            }
        }
        VmMetamethodContinuation::CompleteSet { .. } => {}
        VmMetamethodContinuation::Compare {
            expected,
            invert_result,
            ..
        } => {
            let mut comparison = !result.is_false();
            if invert_result {
                comparison = !comparison;
            }
            if comparison != expected {
                l.current_call_info_mut().savedpc = pc.checked_add(1);
            }
        }
        VmMetamethodContinuation::Concat {
            destination,
            next_slot,
            end_slot,
            stack_limit,
            ..
        } => {
            ensure_stack_slot(l, destination);
            if let Some(slot) = l.stack.at_mut(destination) {
                *slot = result;
            }
            if next_slot <= end_slot {
                for slot in next_slot..=end_slot {
                    let lhs = l.stack.at(destination).cloned().unwrap_or(Value::Nil);
                    let rhs = l.stack.at(slot).cloned().unwrap_or(Value::Nil);
                    match exec_concat_into(
                        l,
                        gc,
                        &lhs,
                        &rhs,
                        destination,
                        VmMetamethodContinuation::Concat {
                            proto,
                            pc,
                            destination,
                            next_slot: slot.saturating_add(1),
                            end_slot,
                            stack_limit,
                        },
                    )? {
                        VmMetamethodOutcome::Ready(()) => {}
                        VmMetamethodOutcome::Suspended(request_id) => {
                            return Ok(VmExit::NativeRequest(request_id));
                        }
                    }
                }
            }
        }
    }
    Ok(VmExit::Complete(ExecResult::Returned))
}

pub(crate) fn finish_lua_return_after_debug_hook(
    l: &mut LuaState,
    call_info: &crate::state::CallInfo,
    results: Vec<Value>,
) -> Result<VmExit, RuntimeError> {
    let current = l.current_call_info();
    if current.func != call_info.func
        || current.base != call_info.base
        || current.proto != call_info.proto
        || current.nresults != call_info.nresults
    {
        return Err(RuntimeError::new(
            "debug-hook Lua return continuation no longer matches its frame",
        ));
    }
    let result_count = results.len();
    if result_count > 0 {
        ensure_stack_slot(l, call_info.func + result_count - 1);
    }
    for (index, value) in results.into_iter().enumerate() {
        if let Some(destination) = l.stack.at_mut(call_info.func + index) {
            *destination = value;
        }
    }
    // The result vector was already sized to the caller-visible wanted count.
    let new_top = call_info.func + result_count;
    for slot in new_top..l.stack.size() {
        if let Some(value) = l.stack.at_mut(slot) {
            *value = Value::Nil;
        }
    }
    l.top = new_top;
    l.pop_call_info();
    Ok(VmExit::Complete(ExecResult::Returned))
}

fn execute_nested_proto_at(
    l: &mut LuaState,
    caller_proto: GcRef<Proto>,
    caller_pc: usize,
    callee_proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    let overflow = stack_overflow_error(l, gc, caller_proto, caller_pc);
    execute_counted_proto(l, callee_proto, gc, overflow)
}

fn execute_counted_proto(
    l: &mut LuaState,
    proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
    overflow: RuntimeError,
) -> Result<VmExit, RuntimeError> {
    if l.nccalls >= MAX_CALLS {
        return Err(overflow);
    }
    l.nccalls += 1;
    let result = execute_proto(l, proto, gc);
    l.nccalls -= 1;
    result
}

fn unwind_lua_frames_to(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    target_ci: usize,
) -> Result<(), RuntimeError> {
    discard_instruction_diffs_above_frame(l, target_ci);
    while l.current_ci > target_ci {
        let base = l.current_call_info().base;
        close_state_upvalues(l, gc, base)?;
        l.pop_call_info();
    }
    Ok(())
}

/// Finish the eager unwind required when an unprotected error escapes the
/// main state. Nested frames close while they are popped, then the root frame
/// closes the Upvalues owned by its register window before the top-level state
/// reset clears the stack.
pub(crate) fn unwind_top_level_lua_error(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
) -> Result<(), RuntimeError> {
    unwind_lua_frames_to(l, gc, 0)?;
    let root_base = l.current_call_info().base;
    close_state_upvalues(l, gc, root_base)
}

/// Call a Lua value from host/stdlib code and publish its results synchronously.
///
/// This mirrors the VM CALL path but restores the caller's stack window before
/// invoking `publish`. Collectable results remain exact-id temporary roots for
/// the entire callback, so they cannot escape through an unrooted `Vec<Value>`.
pub fn call_value_with_results(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    func: Value,
    args: &[Value],
    wanted_results: Option<usize>,
    publish: impl FnOnce(&mut LuaState, &mut GarbageCollector, &[Value]),
) -> Result<(), RuntimeError> {
    let saved_ci = l.current_ci;
    let saved_top = l.top;
    let call_pos = saved_top;
    ensure_stack_slot(l, call_pos + args.len());
    if let Some(dst) = l.stack.at_mut(call_pos) {
        *dst = func.clone();
    }
    for (i, arg) in args.iter().enumerate() {
        if let Some(dst) = l.stack.at_mut(call_pos + 1 + i) {
            *dst = arg.clone();
        }
    }
    l.top = call_pos + 1 + args.len();

    let result = call_value_at_stack(l, gc, call_pos, args.len(), wanted_results)
        .map(|()| collect_call_results(l, call_pos));

    unwind_lua_frames_to(l, gc, saved_ci)?;
    let results = match result {
        Ok(results) => results,
        Err(error) => {
            l.top = saved_top;
            return Err(error);
        }
    };

    let publication = gc.with_publication(|transaction| {
        // SAFETY: the callback is scoped to this function. Every production
        // caller either consumes scalar/byte data synchronously or installs
        // the supplied Values in the Lua stack/table graph before returning.
        unsafe {
            transaction.publish_value_slice(&results, |gc, results| {
                l.top = saved_top;
                publish(l, gc, results)
            })
        }
    });
    match publication {
        Ok(()) => Ok(()),
        Err(error) => {
            l.top = saved_top;
            Err(RuntimeError::new(format!(
                "VM: invalid protected call result: {error}"
            )))
        }
    }
}

pub fn start_lua_call_at_stack(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    func_pos: usize,
    nargs: usize,
    wanted_results: Option<usize>,
) -> Result<(), RuntimeError> {
    let func = l.stack.at(func_pos).cloned().unwrap_or(Value::Nil);
    let Value::Function(func_ref) = func else {
        return Err(RuntimeError::new(format!(
            "attempt to call a {} value",
            value_type_name(&func)
        )));
    };

    let Some(callee_proto_ref) = inspect_function(gc, func_ref)?.0 else {
        return Err(RuntimeError::new("coroutine entry must be a Lua function"));
    };

    let new_base = func_pos + 1;
    let (varargs, max_stack_size) =
        prepare_lua_varargs(l, gc, callee_proto_ref, new_base, nargs as i32)?;
    let ci = l.push_call_info();
    ci.func = func_pos;
    ci.base = new_base;
    ci.top = new_base + max_stack_size;
    ci.nresults = wanted_results.map_or(LUA_MULTRET, |n| n as i32);
    ci.nargs = nargs as i32;
    ci.varargs = varargs;
    ci.savedpc = None;
    ci.proto = Some(callee_proto_ref);
    ci.tailcalls = 0;

    Ok(())
}

pub(crate) fn dispatch_started_lua_call_hook(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
) -> Result<Option<NativeRequestId>, RuntimeError> {
    let proto = l.current_call_info().proto.ok_or_else(|| {
        RuntimeError::new("started Lua call lost its Proto before debug-hook dispatch")
    })?;
    publish_debug_hook_event(
        l,
        gc,
        "call",
        None,
        DebugHookContinuation::LuaCall { proto },
    )
}

pub(crate) fn start_finalizer_callback(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    function: Value,
    userdata: GcRef<lua_core::userdata::Userdata>,
    stop_ci: usize,
) -> Result<VmExit, RuntimeError> {
    if l.current_ci != stop_ci {
        return Err(RuntimeError::new(
            "VM: finalizer callback started at an unexpected frame depth",
        ));
    }
    // The deferred collectgarbage native frame may have lowered logical `top`
    // into the caller's register window. Use the frame's reserved boundary as
    // scratch space so callback arguments cannot overwrite still-live caller
    // locals. Runtime restores the original logical top after the callback.
    let func_pos = l.current_call_info().top.max(l.top);
    l.top = func_pos;
    l.push_value(function.clone());
    l.push_value(Value::Userdata(userdata));
    let Value::Function(function_ref) = function else {
        return Err(RuntimeError::new(
            "attempt to call a non-function __gc value",
        ));
    };
    let callable = gc
        .with_ref(function_ref, native_callable)
        .map_err(|error| RuntimeError::new(format!("invalid __gc function: {error}")))?;
    match callable {
        Some(callable) => call_native_function(l, gc, func_pos, 1, Some(0), callable),
        None => {
            start_lua_call_at_stack(l, gc, func_pos, 1, Some(0))?;
            if let Some(request_id) = dispatch_started_lua_call_hook(l, gc)? {
                return Ok(VmExit::NativeRequest(request_id));
            }
            resume_finalizer_callback(l, gc, stop_ci)
        }
    }
}

pub(crate) fn resume_finalizer_callback(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stop_ci: usize,
) -> Result<VmExit, RuntimeError> {
    loop {
        if l.current_ci <= stop_ci {
            return Ok(VmExit::Complete(ExecResult::Returned));
        }
        let proto = l
            .current_call_info()
            .proto
            .or_else(|| current_lua_proto(l, gc))
            .ok_or_else(|| RuntimeError::new("finalizer frame has no Lua proto"))?;
        // The callback may be resuming after a nested Lua return or a sealed
        // Runtime-native request. Reuse the VM's saved-PC continuation path
        // instead of restarting the current Proto at PC 0.
        l.status = ThreadStatus::Yield;
        match execute_proto(l, proto, gc) {
            Ok(VmExit::Complete(ExecResult::Returned)) => {}
            Ok(VmExit::Complete(ExecResult::Yielded)) => {
                unwind_lua_frames_to(l, gc, stop_ci)?;
                return Err(RuntimeError::new("attempt to yield from a __gc metamethod"));
            }
            Ok(VmExit::NativeRequest(id)) => return Ok(VmExit::NativeRequest(id)),
            Ok(VmExit::UpvalueAccess(request)) => return Ok(VmExit::UpvalueAccess(request)),
            Ok(VmExit::AutomaticGc) => return Ok(VmExit::AutomaticGc),
            Err(error) => {
                unwind_lua_frames_to(l, gc, stop_ci)?;
                return Err(error);
            }
        }
    }
}

pub(crate) fn abort_finalizer_callback(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stop_ci: usize,
    saved_top: usize,
) -> Result<(), RuntimeError> {
    unwind_lua_frames_to(l, gc, stop_ci)?;
    l.top = saved_top;
    Ok(())
}

pub(crate) fn start_runtime_callback_target(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    function: Value,
    args: &[Value],
    stop_ci: usize,
    call_pos: usize,
    wanted_results: Option<usize>,
) -> Result<VmExit, RuntimeError> {
    if l.current_ci != stop_ci {
        return Err(RuntimeError::new(
            "VM: Runtime callback started at an unexpected frame depth",
        ));
    }
    l.status = ThreadStatus::Ok;
    l.top = call_pos;
    ensure_stack_slot(l, call_pos + args.len());
    if let Some(slot) = l.stack.at_mut(call_pos) {
        *slot = function.clone();
    }
    for (index, argument) in args.iter().enumerate() {
        if let Some(slot) = l.stack.at_mut(call_pos + 1 + index) {
            *slot = argument.clone();
        }
    }
    l.top = call_pos + 1 + args.len();

    let Value::Function(function_ref) = function else {
        return Err(RuntimeError::new(format!(
            "attempt to call a {} value",
            value_type_name(&function)
        )));
    };
    if let Some(callable) = inspect_function(gc, function_ref)?.1 {
        return call_native_function(l, gc, call_pos, args.len(), wanted_results, callable);
    }

    start_lua_call_at_stack(l, gc, call_pos, args.len(), wanted_results)?;
    if let Some(request_id) = dispatch_started_lua_call_hook(l, gc)? {
        return Ok(VmExit::NativeRequest(request_id));
    }
    resume_runtime_callback(l, gc, stop_ci)
}

pub(crate) fn resume_runtime_callback(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stop_ci: usize,
) -> Result<VmExit, RuntimeError> {
    loop {
        if l.current_ci == stop_ci {
            return Ok(VmExit::Complete(ExecResult::Returned));
        }
        if l.current_ci < stop_ci {
            return Err(RuntimeError::new(
                "VM: Runtime callback resumed below its caller boundary",
            ));
        }
        let proto = l
            .current_call_info()
            .proto
            .or_else(|| current_lua_proto(l, gc))
            .ok_or_else(|| RuntimeError::new("Runtime callback frame has no Lua proto"))?;
        l.status = ThreadStatus::Yield;
        match execute_proto(l, proto, gc)? {
            VmExit::Complete(ExecResult::Returned) => {}
            VmExit::Complete(ExecResult::Yielded) => {
                return Ok(VmExit::Complete(ExecResult::Yielded));
            }
            VmExit::NativeRequest(id) => return Ok(VmExit::NativeRequest(id)),
            VmExit::UpvalueAccess(request) => return Ok(VmExit::UpvalueAccess(request)),
            VmExit::AutomaticGc => return Ok(VmExit::AutomaticGc),
        }
    }
}

pub(crate) fn finish_runtime_callback_turn(
    l: &mut LuaState,
    stop_ci: usize,
    call_pos: usize,
    saved_top: usize,
) -> Result<Vec<Value>, RuntimeError> {
    if l.current_ci != stop_ci {
        return Err(RuntimeError::new(
            "VM: Runtime callback completed above its caller boundary",
        ));
    }
    if l.top < call_pos {
        return Err(RuntimeError::new(
            "VM: Runtime callback result window moved below its call slot",
        ));
    }
    let results = collect_call_results(l, call_pos);
    l.top = saved_top;
    Ok(results)
}

/// Finish a protected target while matching the fixed C++ pcall scratch-window
/// cleanup. Generic Runtime callbacks retain their inactive register window;
/// clearing it there can erase caller globals/locals still owned by a wider CI.
pub(crate) fn finish_protected_callback_turn(
    l: &mut LuaState,
    stop_ci: usize,
    call_pos: usize,
    saved_top: usize,
) -> Result<Vec<Value>, RuntimeError> {
    if l.current_ci != stop_ci {
        return Err(RuntimeError::new(
            "VM: protected callback completed above its caller boundary",
        ));
    }
    if l.top < call_pos {
        return Err(RuntimeError::new(
            "VM: protected callback result window moved below its call slot",
        ));
    }
    let results = collect_call_results(l, call_pos);
    let clear_end = l.top.min(l.stack.size());
    for slot in saved_top.min(clear_end)..clear_end {
        if let Some(value) = l.stack.at_mut(slot) {
            *value = Value::Nil;
        }
    }
    l.top = saved_top;
    Ok(results)
}

pub(crate) fn abort_runtime_callback_turn(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stop_ci: usize,
    saved_top: usize,
) -> Result<(), RuntimeError> {
    unwind_lua_frames_to(l, gc, stop_ci)?;
    l.top = saved_top;
    l.status = ThreadStatus::Ok;
    Ok(())
}

pub fn resume_lua_thread(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    loop {
        if l.current_ci == 0 {
            return Ok(VmExit::Complete(ExecResult::Returned));
        }

        let proto_ref = l
            .current_call_info()
            .proto
            .or_else(|| current_lua_proto(l, gc))
            .ok_or_else(|| RuntimeError::new("coroutine frame has no Lua proto"))?;
        gc.validate_ref(proto_ref).map_err(invalid_proto_error)?;

        l.status = ThreadStatus::Yield;
        let result = execute_proto(l, proto_ref, gc)?;
        match result {
            VmExit::Complete(ExecResult::Yielded) => {
                return Ok(VmExit::Complete(ExecResult::Yielded));
            }
            VmExit::Complete(ExecResult::Returned) => {
                if l.current_ci == 0 {
                    return Ok(VmExit::Complete(ExecResult::Returned));
                }
            }
            VmExit::NativeRequest(id) => return Ok(VmExit::NativeRequest(id)),
            VmExit::UpvalueAccess(request) => return Ok(VmExit::UpvalueAccess(request)),
            VmExit::AutomaticGc => return Ok(VmExit::AutomaticGc),
        }
    }
}

fn call_value_at_stack(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    func_pos: usize,
    nargs: usize,
    wanted_results: Option<usize>,
) -> Result<(), RuntimeError> {
    let func = l.stack.at(func_pos).cloned().unwrap_or(Value::Nil);
    let Value::Function(func_ref) = func else {
        return Err(RuntimeError::new(format!(
            "attempt to call a {} value",
            value_type_name(&func)
        )));
    };

    let (callee_proto, callable) = inspect_function(gc, func_ref)?;
    if let Some(callable) = callable {
        match call_native_function(l, gc, func_pos, nargs, wanted_results, callable)? {
            VmExit::Complete(ExecResult::Returned) => {}
            VmExit::Complete(ExecResult::Yielded) => {
                return Err(RuntimeError::new("cannot yield across pcall"));
            }
            VmExit::NativeRequest(id) => {
                if !l.retarget_native_request_to_protected_resume(id) {
                    return Err(RuntimeError::new(
                        "VM: failed to retarget protected Runtime-native request",
                    ));
                }
                return Err(RuntimeError::native_request_suspend());
            }
            VmExit::UpvalueAccess(_) => {
                return Err(RuntimeError::new(
                    "open Upvalue access cannot cross a protected helper boundary yet",
                ));
            }
            VmExit::AutomaticGc => {
                return Err(RuntimeError::new(
                    "automatic GC checkpoint crossed a protected helper boundary",
                ));
            }
        }
        return Ok(());
    }

    let Some(callee_proto_ref) = callee_proto else {
        return Err(RuntimeError::new("Lua function has no proto"));
    };

    let saved_ci = l.current_ci;
    let new_base = func_pos + 1;
    let (varargs, max_stack_size) =
        prepare_lua_varargs(l, gc, callee_proto_ref, new_base, nargs as i32)?;
    let ci = l.push_call_info();
    ci.func = func_pos;
    ci.base = new_base;
    ci.top = new_base + max_stack_size;
    ci.nresults = wanted_results.map_or(LUA_MULTRET, |n| n as i32);
    ci.nargs = nargs as i32;
    ci.varargs = varargs;
    ci.savedpc = None;
    ci.proto = Some(callee_proto_ref);
    ci.tailcalls = 0;

    gc.suspend_automatic_checkpoints();
    let protected_result =
        execute_counted_proto(l, callee_proto_ref, gc, RuntimeError::new("stack overflow"));
    gc.resume_automatic_checkpoints();
    match protected_result {
        Ok(VmExit::Complete(ExecResult::Returned)) => {}
        Ok(VmExit::Complete(ExecResult::Yielded)) => {
            unwind_lua_frames_to(l, gc, saved_ci)?;
            return Err(RuntimeError::new("cannot yield across pcall"));
        }
        Ok(VmExit::NativeRequest(_)) => {
            return Err(RuntimeError::new(
                "runtime-native request cannot cross a protected helper boundary yet",
            ));
        }
        Ok(VmExit::UpvalueAccess(_)) => {
            return Err(RuntimeError::new(
                "open Upvalue access cannot cross a protected helper boundary yet",
            ));
        }
        Ok(VmExit::AutomaticGc) => {
            return Err(RuntimeError::new(
                "automatic GC checkpoint crossed a protected helper boundary",
            ));
        }
        Err(e) => {
            if l.current_thread.is_none() {
                unwind_lua_frames_to(l, gc, saved_ci)?;
            }
            return Err(e);
        }
    }

    if l.current_ci != saved_ci {
        unwind_lua_frames_to(l, gc, saved_ci)?;
        return Err(RuntimeError::new("VM: helper call frame imbalance"));
    }
    Ok(())
}

fn collect_call_results(l: &LuaState, call_pos: usize) -> Vec<Value> {
    (call_pos..l.top)
        .map(|idx| l.stack.at(idx).cloned().unwrap_or(Value::Nil))
        .collect()
}

fn c_function_display_name(l: &LuaState, gc: &GarbageCollector, func_pos: usize) -> String {
    let Some(Value::Function(func_ref)) = l.stack.at(func_pos).cloned() else {
        return "<unknown>".to_string();
    };
    let Some(global_ref) = l.global_table else {
        return format!("function: {:p}", func_ref.as_ptr());
    };
    let display_name = gc.with_ref(global_ref, |global| {
        if let Some(name) = find_function_name_in_table(gc, global, func_ref) {
            return Some(name);
        }
        for (key, value) in global.hash_entries() {
            let Value::String(lib_name_ref) = key else {
                continue;
            };
            let Value::Table(table_ref) = value else {
                continue;
            };
            let name = gc
                .with_ref(*table_ref, |table| {
                    find_function_name_in_table(gc, table, func_ref)
                })
                .ok()
                .flatten();
            if let Some(name) = name {
                let lib_name = gc
                    .with_string_bytes(*lib_name_ref, |bytes| {
                        String::from_utf8_lossy(bytes).into_owned()
                    })
                    .ok()
                    .unwrap_or_default();
                return Some(format!("{lib_name}.{name}"));
            }
        }
        None
    });
    display_name
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("function: {:p}", func_ref.as_ptr()))
}

fn find_function_name_in_table(
    gc: &GarbageCollector,
    table: &Table,
    func_ref: GcRef<Function>,
) -> Option<String> {
    for (key, value) in table.hash_entries() {
        if let (Value::String(name_ref), Value::Function(value_ref)) = (key, value)
            && *value_ref == func_ref
        {
            return gc
                .with_string_bytes(*name_ref, |bytes| {
                    String::from_utf8_lossy(bytes).into_owned()
                })
                .ok();
        }
    }
    None
}

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Boolean(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Table(_) => "table",
        Value::Function(_) => "function",
        Value::Userdata(_) => "userdata",
        Value::Thread(_) => "thread",
        Value::LightUserdata(_) => "lightuserdata",
    }
}

fn runtime_error_at(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
    message: impl Into<String>,
) -> RuntimeError {
    let (source_ref, line) = gc
        .with_ref(proto, |proto| (proto.source(), proto.line(pc)))
        .unwrap_or((None, 0));
    let source = source_ref
        .and_then(|source_ref| {
            gc.with_string_bytes(source_ref, chunk_name_for_diagnostic)
                .ok()
        })
        .unwrap_or_else(|| "?".to_string());
    RuntimeError::new(format!("{}:{}: {}", source, line, message.into()))
}

fn stack_overflow_error(
    l: &LuaState,
    gc: &GarbageCollector,
    caller_proto: GcRef<Proto>,
    caller_pc: usize,
) -> RuntimeError {
    let recursive_line = gc
        .with_ref(caller_proto, |proto| proto.line(caller_pc))
        .unwrap_or(0);
    let mut message = format!("?:{}: stack overflow", recursive_line);
    for _ in 0..20 {
        message.push_str(&format!("\n?:{}: in function 'y'", recursive_line));
    }
    if let Some(line) = first_non_recursive_caller_line(l, gc) {
        message.push_str(&format!("\n?:{}: in function 'g'", line));
    } else {
        message.push_str(&format!("\n?:{}: in function", recursive_line));
    }
    RuntimeError::new(message)
}

fn first_non_recursive_caller_line(l: &LuaState, gc: &GarbageCollector) -> Option<i32> {
    let mut top_func: Option<GcRef<Function>> = None;
    for ci in l.call_stack.iter().take(l.current_ci + 1).rev() {
        let Value::Function(func_ref) = l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) else {
            continue;
        };
        if top_func.is_none() {
            top_func = Some(func_ref);
            continue;
        }
        if Some(func_ref) == top_func {
            continue;
        }
        let proto_ref = ci.proto?;
        return ci
            .savedpc
            .and_then(|pc| gc.with_ref(proto_ref, |proto| proto.line(pc)).ok());
    }
    None
}

fn should_error_index(l: &LuaState, gc: &GarbageCollector, value: &Value) -> bool {
    !matches!(value, Value::Table(_) | Value::String(_))
        && value_metatable(l, gc, value)
            .and_then(|mt| lookup_metamethod(l, gc, mt, "__index"))
            .is_none()
}

fn should_error_newindex(l: &LuaState, gc: &GarbageCollector, value: &Value) -> bool {
    !matches!(value, Value::Table(_))
        && value_metatable(l, gc, value)
            .and_then(|mt| lookup_metamethod(l, gc, mt, "__newindex"))
            .is_none()
}

fn index_error_message(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
    reg: usize,
    value: &Value,
) -> String {
    let type_desc = value_type_name(value);
    gc.with_ref(proto, |proto| {
        describe_register_for_index(gc, proto, pc, reg, proto.constants(), 8)
    })
    .ok()
    .flatten()
    .map(|name| format!("attempt to index {name} (a {type_desc} value)"))
    .unwrap_or_else(|| format!("attempt to index a {type_desc} value"))
}

fn arith_error_message(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
    lhs: (i32, &Value),
    rhs: (i32, &Value),
) -> String {
    let (bad_rk, bad_value) = if to_arith_number(gc, lhs.1).is_none() {
        lhs
    } else {
        rhs
    };
    gc.with_ref(proto, |proto| {
        operand_error_message(
            gc,
            proto,
            pc,
            bad_rk,
            proto.constants(),
            bad_value,
            "attempt to perform arithmetic on",
        )
    })
    .unwrap_or_else(|_| "attempt to perform arithmetic on a non-number value".to_string())
}

fn unm_error_message(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
    rk: usize,
    value: &Value,
) -> String {
    gc.with_ref(proto, |proto| {
        operand_error_message(
            gc,
            proto,
            pc,
            rk as i32,
            proto.constants(),
            value,
            "attempt to perform arithmetic on",
        )
    })
    .unwrap_or_else(|_| "attempt to perform arithmetic on a non-number value".to_string())
}

fn describe_register_at(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
    reg: usize,
    depth: usize,
) -> Option<String> {
    gc.with_ref(proto, |proto| {
        describe_register(gc, proto, pc, reg, proto.constants(), depth)
    })
    .ok()
    .flatten()
}

fn operand_error_message(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    rk: i32,
    constants: &[Value],
    value: &Value,
    prefix: &str,
) -> String {
    let type_desc = value_type_name(value);
    describe_operand(gc, proto, pc, rk, constants, 8)
        .map(|name| format!("{prefix} {name} (a {type_desc} value)"))
        .unwrap_or_else(|| format!("{prefix} a non-number value"))
}

fn describe_operand(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    rk: i32,
    constants: &[Value],
    depth: usize,
) -> Option<String> {
    if opcode::is_k(rk) {
        None
    } else {
        describe_register(gc, proto, pc, rk as usize, constants, depth)
    }
}

fn describe_register(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    depth: usize,
) -> Option<String> {
    describe_register_impl(gc, proto, pc, reg, constants, depth, true)
}

fn describe_register_for_index(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    depth: usize,
) -> Option<String> {
    describe_register_impl(gc, proto, pc, reg, constants, depth, false)
}

fn describe_register_impl(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    depth: usize,
    respect_name_barrier: bool,
) -> Option<String> {
    if depth == 0 {
        return None;
    }

    if let Some(name) = local_name_for_reg(gc, proto, reg, pc) {
        return Some(format!("local '{name}'"));
    }

    let code = proto.code();
    for cursor in (0..pc).rev().take(16) {
        let inst = code[cursor];
        let op = opcode::get_opcode(inst);
        let a = opcode::get_arg_a(inst) as usize;
        if a != reg {
            continue;
        }

        match op {
            OpCode::MOVE => {
                let source = opcode::get_arg_b(inst) as usize;
                return describe_register_impl(
                    gc,
                    proto,
                    cursor,
                    source,
                    constants,
                    depth - 1,
                    respect_name_barrier,
                );
            }
            OpCode::GETUPVAL => {
                let upvalue = opcode::get_arg_b(inst) as usize;
                return proto
                    .upvalue_name(upvalue)
                    .and_then(|name| gc_string_lossy(gc, name))
                    .map(|name| format!("upvalue '{name}'"));
            }
            OpCode::GETGLOBAL => {
                if respect_name_barrier
                    && cursor > 0
                    && is_name_barrier(opcode::get_opcode(code[cursor - 1]))
                    && has_prior_write_to_register(code, cursor, reg)
                {
                    return None;
                }
                let bx = opcode::get_arg_bx(inst) as usize;
                return constant_string(gc, constants, bx).map(|name| format!("global '{name}'"));
            }
            OpCode::GETTABLE => {
                let key = opcode::get_arg_c(inst);
                if let Some(name) = rk_string(gc, constants, key) {
                    return Some(format!("field '{name}'"));
                }
                return None;
            }
            OpCode::SELF => {
                let key = opcode::get_arg_c(inst);
                if let Some(name) = rk_string(gc, constants, key) {
                    return Some(format!("method '{name}'"));
                }
                return None;
            }
            _ => return None,
        }
    }

    None
}

fn is_name_barrier(op: OpCode) -> bool {
    matches!(op, OpCode::JMP | OpCode::TEST | OpCode::TESTSET)
}

fn has_prior_write_to_register(
    code: &[lua_core::proto::Instruction],
    cursor: usize,
    reg: usize,
) -> bool {
    for inst in code[..cursor].iter().rev().take(12) {
        let op = opcode::get_opcode(*inst);
        if instruction_writes_register(op, *inst, reg) {
            return true;
        }
    }
    false
}

fn instruction_writes_register(op: OpCode, inst: lua_core::proto::Instruction, reg: usize) -> bool {
    match op {
        OpCode::MOVE
        | OpCode::LOADK
        | OpCode::LOADBOOL
        | OpCode::GETUPVAL
        | OpCode::GETGLOBAL
        | OpCode::GETTABLE
        | OpCode::NEWTABLE
        | OpCode::SELF
        | OpCode::ADD
        | OpCode::SUB
        | OpCode::MUL
        | OpCode::DIV
        | OpCode::MOD
        | OpCode::POW
        | OpCode::UNM
        | OpCode::NOT
        | OpCode::LEN
        | OpCode::CONCAT
        | OpCode::CALL
        | OpCode::TAILCALL
        | OpCode::VARARG => opcode::get_arg_a(inst) as usize == reg,
        OpCode::LOADNIL => {
            let a = opcode::get_arg_a(inst) as usize;
            let b = opcode::get_arg_b(inst) as usize;
            (a..=b).contains(&reg)
        }
        _ => false,
    }
}

fn local_name_for_reg(
    gc: &GarbageCollector,
    proto: &Proto,
    reg: usize,
    pc: usize,
) -> Option<String> {
    let pc = pc as i32;
    for idx in (0..proto.loc_var_count()).rev() {
        let loc = proto.loc_var(idx);
        if loc.reg == reg as i32
            && loc.startpc <= pc
            && pc < loc.endpc
            && let Some(name_ref) = loc.varname
        {
            return gc_string_lossy(gc, name_ref);
        }
    }
    None
}

fn rk_string(gc: &GarbageCollector, constants: &[Value], rk: i32) -> Option<String> {
    if opcode::is_k(rk) {
        constant_string(gc, constants, opcode::index_k(rk) as usize)
    } else {
        None
    }
}

fn constant_string(gc: &GarbageCollector, constants: &[Value], idx: usize) -> Option<String> {
    match constants.get(idx) {
        Some(Value::String(name_ref)) => gc_string_lossy(gc, *name_ref),
        _ => None,
    }
}

fn gc_string_lossy(gc: &GarbageCollector, name_ref: GcRef<GcString>) -> Option<String> {
    gc.with_string_bytes(name_ref, |bytes| {
        String::from_utf8_lossy(bytes).into_owned()
    })
    .ok()
}

// ═══════════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════════

fn ensure_stack_slot(l: &mut LuaState, index: usize) {
    if l.stack.size() <= index {
        l.stack.set_top(index + 1);
    }
}

fn collect_varargs(l: &LuaState, base: usize, nargs: i32, fixed_params: usize) -> Vec<Value> {
    let actual_args = nargs.max(0) as usize;
    if actual_args <= fixed_params {
        return Vec::new();
    }

    (fixed_params..actual_args)
        .map(|i| l.stack.at(base + i).cloned().unwrap_or(Value::Nil))
        .collect()
}

fn fill_missing_fixed_params(l: &mut LuaState, base: usize, nargs: i32, fixed_params: usize) {
    let actual_args = nargs.max(0) as usize;
    if actual_args >= fixed_params {
        return;
    }
    if fixed_params > 0 {
        ensure_stack_slot(l, base + fixed_params - 1);
    }
    for i in actual_args..fixed_params {
        if let Some(dst) = l.stack.at_mut(base + i) {
            *dst = Value::Nil;
        }
    }
}

fn prepare_lua_varargs(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    proto: GcRef<Proto>,
    base: usize,
    nargs: i32,
) -> Result<(Vec<Value>, usize), RuntimeError> {
    let (fixed_params, is_vararg, vararg_flags, max_stack_size) = gc
        .with_ref(proto, |proto| {
            (
                proto.num_params() as usize,
                proto.is_vararg(),
                proto.vararg_flags(),
                proto.max_stack_size() as usize,
            )
        })
        .map_err(invalid_proto_error)?;
    let varargs = if is_vararg {
        collect_varargs(l, base, nargs, fixed_params)
    } else {
        Vec::new()
    };

    fill_missing_fixed_params(l, base, nargs, fixed_params);
    if max_stack_size > 0 {
        ensure_stack_slot(l, base + max_stack_size - 1);
    }
    for slot in fixed_params..max_stack_size {
        if let Some(value) = l.stack.at_mut(base + slot) {
            *value = Value::Nil;
        }
    }

    if vararg_flags & VARARG_NEEDSARG != 0 {
        install_arg_table(l, gc, base + fixed_params, &varargs);
    }

    Ok((varargs, max_stack_size))
}

fn install_arg_table(l: &mut LuaState, gc: &mut GarbageCollector, slot: usize, varargs: &[Value]) {
    ensure_stack_slot(l, slot);

    gc.with_publication(|transaction| {
        let table = transaction.alloc(Table::new());
        for (idx, value) in varargs.iter().enumerate() {
            transaction
                .set_table_entry(&table, &Value::Number((idx + 1) as f64), value)
                .expect("VM vararg values belong to the active collector");
        }
        let n_key = rooted_vm_bytes(l, transaction, b"n")
            .expect("VM vararg Table requires the active Runtime StringPool");
        transaction
            .set_table_value(&table, &n_key, &Value::Number(varargs.len() as f64))
            .expect("VM vararg count publication remains collector-valid");

        // SAFETY: the callback installs the completed compatibility table in
        // an active VM stack slot before releasing its temporary root.
        unsafe {
            transaction.publish_table_value(table, |value| {
                if let Some(dst) = l.stack.at_mut(slot) {
                    *dst = value;
                }
            })
        }
        .expect("new VM vararg Table remains registered")
    });
}

#[derive(Clone, Copy)]
enum NativeCallable {
    C(CFunction),
    Runtime(RuntimeNativeFunction),
}

fn native_callable(function: &Function) -> Option<NativeCallable> {
    function
        .runtime_native_function()
        .map(NativeCallable::Runtime)
        .or_else(|| function.c_function().map(NativeCallable::C))
}

fn inspect_function(
    gc: &GarbageCollector,
    function: GcRef<Function>,
) -> Result<(Option<GcRef<Proto>>, Option<NativeCallable>), RuntimeError> {
    gc.with_ref(function, |function| {
        (function.proto(), native_callable(function))
    })
    .map_err(|error| RuntimeError::new(format!("VM: invalid Function: {error}")))
}

fn call_native_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    func_pos: usize,
    nargs: usize,
    wanted_results: Option<usize>,
    callable: NativeCallable,
) -> Result<VmExit, RuntimeError> {
    let saved_ci = l.current_ci;
    let saved_top = l.top;
    let caller_proto = l.current_call_info().proto;
    let caller_pc = l.current_call_info().savedpc;
    let frame = DebugHookNativeCall {
        func_pos,
        nargs,
        wanted_results,
        saved_ci,
        saved_top,
        caller_proto,
        caller_pc,
        continuation: DeferredVmContinuation::Call,
    };
    let ci_top = {
        let ci = l.push_call_info();
        ci.func = func_pos;
        ci.base = func_pos + 1;
        ci.top = func_pos + 1 + nargs + 20;
        ci.nresults = wanted_results.map_or(-1, |n| n as i32);
        ci.nargs = nargs as i32;
        ci.savedpc = None;
        ci.proto = None;
        ci.tailcalls = 0;
        ci.top
    };

    if l.stack.size() < ci_top {
        l.stack.set_top(ci_top);
    }
    let argument_top = func_pos + 1 + nargs;
    if matches!(callable, NativeCallable::C(_)) {
        // The fixed C++ C frame initializes its unused register window. This
        // is observable in full-register snapshots even though it lies above
        // the callback's logical argument top.
        record_trace_nil_range(l, argument_top, ci_top);
    }
    l.top = argument_top;

    match publish_debug_hook_event(
        l,
        gc,
        "call",
        None,
        DebugHookContinuation::NativeCall {
            frame: frame.clone(),
        },
    ) {
        Ok(Some(request_id)) => return Ok(VmExit::NativeRequest(request_id)),
        Ok(None) => {}
        Err(error) => {
            unwind_lua_frames_to(l, gc, saved_ci)?;
            l.top = saved_top;
            return Err(error);
        }
    }

    invoke_native_call_after_debug_hook(l, gc, frame, callable)
}

pub(crate) fn resume_native_call_after_debug_hook(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    frame: DebugHookNativeCall,
) -> Result<VmExit, RuntimeError> {
    if l.current_ci != frame.saved_ci + 1 {
        return Err(RuntimeError::new(
            "debug-hook native-call continuation changed frame depth",
        ));
    }
    let function = l.stack.at(frame.func_pos).cloned().unwrap_or(Value::Nil);
    let Value::Function(function_ref) = function else {
        return Err(RuntimeError::new(
            "debug-hook native-call continuation lost its function",
        ));
    };
    let callable = inspect_function(gc, function_ref)?.1.ok_or_else(|| {
        RuntimeError::new("debug-hook native-call continuation is no longer native")
    })?;
    invoke_native_call_after_debug_hook(l, gc, frame, callable)
}

fn invoke_native_call_after_debug_hook(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    frame: DebugHookNativeCall,
    callable: NativeCallable,
) -> Result<VmExit, RuntimeError> {
    let DebugHookNativeCall {
        func_pos,
        nargs,
        wanted_results,
        saved_ci,
        saved_top,
        caller_proto,
        caller_pc,
        continuation,
    } = frame.clone();
    let nret = match callable {
        NativeCallable::C(cfunc) => {
            let state = NonNull::from(&mut *l);
            let collector = NonNull::from(&mut *gc);
            let strings =
                NonNull::new(l.active_string_pool_ptr().ok_or_else(|| {
                    RuntimeError::new("native callback has no active StringPool")
                })?)
                .expect("active StringPool pointer is non-null");
            // SAFETY: this nested scope is derived from the currently active
            // state/collector reborrows. It suspends the outer collector borrow
            // while the C callback uses LuaState's dynamic scoped services.
            let _callback_scope = unsafe { enter_vm_context(state, collector, strings) };
            // SAFETY: state points to the currently executing LuaState and the
            // ordinary C ABI owns no Runtime state-switch capability.
            unsafe { cfunc(state.as_ptr().cast()) }
        }
        NativeCallable::Runtime(operation) => invoke_runtime_native(l, gc, operation),
    };

    if let Some(request_id) = l.pending_native_request_id() {
        let deferred = DeferredNativeCall {
            request_id,
            func_pos,
            nargs,
            wanted_results,
            saved_ci,
            saved_top,
            caller_proto,
            caller_pc,
            continuation,
            snapshot: crate::native::StateContinuationSnapshot::capture(l),
        };
        if !l.seal_native_request(request_id, deferred) {
            unwind_lua_frames_to(l, gc, saved_ci)?;
            l.top = saved_top;
            return Err(RuntimeError::new(
                "VM: failed to seal Runtime-native request",
            ));
        }
        return Ok(VmExit::NativeRequest(request_id));
    }

    if l.status == ThreadStatus::Yield {
        l.yield_result_base = Some(func_pos);
        l.yield_wanted_results = wanted_results;
        l.pop_call_info();
        l.top = func_pos;
        flush_instruction_diff(l, gc);
        if let (Some(proto), Some(pc)) = (caller_proto, caller_pc) {
            emit_coroutine_yield_trace(l, gc, proto, pc);
        }
        return Ok(VmExit::Complete(ExecResult::Yielded));
    }

    finish_native_call(l, gc, frame, nret)
}

fn finish_native_call(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    frame: DebugHookNativeCall,
    nret: i32,
) -> Result<VmExit, RuntimeError> {
    let func_pos = frame.func_pos;
    let saved_ci = frame.saved_ci;
    let saved_top = frame.saved_top;
    if nret < 0 {
        let native_base = l.current_call_info().base;
        let error_value = if l.top > native_base {
            l.top
                .checked_sub(1)
                .and_then(|idx| l.stack.at(idx))
                .cloned()
        } else {
            None
        };
        let display_name = c_function_display_name(l, gc, func_pos);
        unwind_lua_frames_to(l, gc, saved_ci)?;
        l.top = saved_top;
        return Err(error_value
            .map(|value| RuntimeError::with_value(gc, value))
            .unwrap_or_else(|| {
                RuntimeError::new(format!("C function call failed: {display_name}"))
            }));
    }

    match publish_debug_hook_event(
        l,
        gc,
        "return",
        None,
        DebugHookContinuation::NativeReturn {
            frame: frame.clone(),
            result_count: nret,
        },
    ) {
        Ok(Some(request_id)) => return Ok(VmExit::NativeRequest(request_id)),
        Ok(None) => {}
        Err(error) => {
            unwind_lua_frames_to(l, gc, saved_ci)?;
            l.top = saved_top;
            return Err(error);
        }
    }

    finish_native_call_after_debug_hook(l, gc, frame, nret)
}

pub(crate) fn finish_native_call_after_debug_hook(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    frame: DebugHookNativeCall,
    nret: i32,
) -> Result<VmExit, RuntimeError> {
    let DebugHookNativeCall {
        func_pos,
        nargs: _,
        wanted_results,
        saved_ci,
        saved_top: _,
        caller_proto: _,
        caller_pc,
        continuation,
    } = frame;
    if l.current_ci != saved_ci + 1 || l.current_call_info().func != func_pos {
        return Err(RuntimeError::new(
            "debug-hook native-return continuation changed its call frame",
        ));
    }
    let nret_count = usize::try_from(nret).map_err(|_| {
        RuntimeError::new("debug-hook native-return continuation has a negative result count")
    })?;
    let old_top = l.top;
    let first_result = l.top.saturating_sub(nret_count);
    let wanted_count = wanted_results.unwrap_or(nret_count);
    if wanted_count > 0 {
        ensure_stack_slot(l, func_pos + wanted_count - 1);
    }
    for i in 0..wanted_count {
        let src = if i < nret_count {
            l.stack.at(first_result + i).cloned().unwrap_or(Value::Nil)
        } else {
            Value::Nil
        };
        if let Some(dst) = l.stack.at_mut(func_pos + i) {
            *dst = src;
        }
    }
    let new_top = func_pos + wanted_count;
    record_trace_nil_range(l, new_top, old_top);
    l.pop_call_info();
    l.top = new_top;
    if matches!(continuation, DeferredVmContinuation::TailCall) {
        // The pinned C++ TAILCALL handler closes the caller's Upvalues before
        // invoking any target but continues to the compiler-emitted RETURN
        // after a native target completes. Preserve that frame/PC here.
        close_state_upvalues(l, gc, l.current_call_info().base)?;
    } else {
        apply_vm_continuation(l, gc, &continuation, saved_ci, caller_pc)?;
    }
    Ok(VmExit::Complete(ExecResult::Returned))
}

pub(crate) fn finish_deferred_native_call(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    envelope: ResumeEnvelope,
    response: ResumeResponse,
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;

    l.top = deferred.func_pos + 1 + deferred.nargs;
    let nret = match (envelope, response) {
        (ResumeEnvelope::Resume, ResumeResponse::Success(values)) => {
            l.push_boolean(true);
            let count = values.len() + 1;
            for value in values {
                l.push_value(value);
            }
            count as i32
        }
        (ResumeEnvelope::Resume, ResumeResponse::Error(error)) => {
            l.push_boolean(false);
            l.push_value(error);
            2
        }
        (ResumeEnvelope::Wrap, ResumeResponse::Success(values)) => {
            let count = values.len();
            for value in values {
                l.push_value(value);
            }
            count as i32
        }
        (ResumeEnvelope::Wrap, ResumeResponse::Error(error)) => {
            l.push_value(error);
            -1
        }
        (ResumeEnvelope::ProtectedResume, ResumeResponse::Success(values)) => {
            l.push_boolean(true);
            l.push_boolean(true);
            let count = values.len() + 2;
            for value in values {
                l.push_value(value);
            }
            count as i32
        }
        (ResumeEnvelope::ProtectedResume, ResumeResponse::Error(error)) => {
            l.push_boolean(true);
            l.push_boolean(false);
            l.push_value(error);
            3
        }
        (ResumeEnvelope::ProtectedWrap, ResumeResponse::Success(values)) => {
            l.push_boolean(true);
            let count = values.len() + 1;
            for value in values {
                l.push_value(value);
            }
            count as i32
        }
        (ResumeEnvelope::ProtectedWrap, ResumeResponse::Error(error)) => {
            l.push_boolean(false);
            l.push_value(error);
            2
        }
    };

    finish_native_call(l, gc, native_call_frame_from_deferred(deferred), nret)
}

pub(crate) fn finish_deferred_native_values(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    values: Vec<Value>,
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1 + deferred.nargs;
    let count = values.len();
    for value in values {
        l.push_value(value);
    }
    finish_native_call(
        l,
        gc,
        native_call_frame_from_deferred(deferred),
        count as i32,
    )
}

/// Finish pcall/xpcall values in the native frame's result base.
///
/// Unlike an ordinary deferred native operation, the fixed C++ protected
/// helper executes its target in place and returns its status/value tuple from
/// the first native-frame slot. Preserving that staging point also preserves
/// the oracle's physical scratch-window cleanup and GC roots.
pub(crate) fn finish_deferred_protected_values(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    values: Vec<Value>,
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1;
    let count = values.len();
    for value in values {
        l.push_value(value);
    }
    finish_native_call(
        l,
        gc,
        native_call_frame_from_deferred(deferred),
        count as i32,
    )
}

pub(crate) fn finish_deferred_runtime_result(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    protected: bool,
    response: Result<Vec<Value>, Value>,
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1 + deferred.nargs;
    let nret = match (protected, response) {
        (false, Ok(values)) => {
            let count = values.len();
            for value in values {
                l.push_value(value);
            }
            count as i32
        }
        (false, Err(error)) => {
            l.push_value(error);
            -1
        }
        (true, Ok(values)) => {
            l.push_boolean(true);
            let count = values.len() + 1;
            for value in values {
                l.push_value(value);
            }
            count as i32
        }
        (true, Err(error)) => {
            l.push_boolean(false);
            l.push_value(error);
            2
        }
    };
    finish_native_call(l, gc, native_call_frame_from_deferred(deferred), nret)
}

pub(crate) fn finish_deferred_load_source(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    source: &[u8],
    chunk_name: &[u8],
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1 + deferred.nargs;
    match compile_chunk_function(l, gc, source, chunk_name) {
        Ok(_) => finish_deferred_native_stack_results(l, gc, deferred, 1),
        Err(message) => finish_validated_deferred_load_error(l, gc, deferred, &message),
    }
}

pub(crate) fn finish_deferred_load_error_message(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    message: &str,
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1 + deferred.nargs;
    finish_validated_deferred_load_error(l, gc, deferred, message)
}

pub(crate) fn finish_deferred_gsub_result(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    output: &[u8],
    count: usize,
) -> Result<VmExit, RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1 + deferred.nargs;
    gc.with_publication(|transaction| {
        let output = rooted_vm_bytes(l, transaction, output)?;
        // SAFETY: the result string is installed on the retained native frame
        // before its temporary publication root is released.
        unsafe {
            transaction.publish_string_value(output, |output| {
                l.push_value(output);
            })
        }
    })
    .map_err(|error| RuntimeError::new(format!("VM: invalid string.gsub result: {error}")))?;
    l.push_value(Value::Number(count as f64));
    finish_deferred_native_stack_results(l, gc, deferred, 2)
}

fn finish_validated_deferred_load_error(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    message: &str,
) -> Result<VmExit, RuntimeError> {
    l.push_value(Value::Nil);
    gc.with_publication(|transaction| {
        let message = rooted_vm_bytes(l, transaction, message.as_bytes())?;
        // SAFETY: the message is installed on the active native frame before
        // the publication root is released.
        unsafe {
            transaction.publish_string_value(message, |message| {
                l.push_value(message);
            })
        }
    })
    .map_err(|error| RuntimeError::new(format!("VM: invalid load error result: {error}")))?;
    finish_deferred_native_stack_results(l, gc, deferred, 2)
}

fn finish_deferred_native_stack_results(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    result_count: i32,
) -> Result<VmExit, RuntimeError> {
    let frame = native_call_frame_from_deferred(deferred);
    finish_native_call(l, gc, frame, result_count)
}

fn native_call_frame_from_deferred(deferred: &DeferredNativeCall) -> DebugHookNativeCall {
    DebugHookNativeCall {
        func_pos: deferred.func_pos,
        nargs: deferred.nargs,
        wanted_results: deferred.wanted_results,
        saved_ci: deferred.saved_ci,
        saved_top: deferred.saved_top,
        caller_proto: deferred.caller_proto,
        caller_pc: deferred.caller_pc,
        continuation: deferred.continuation.clone(),
    }
}

pub(crate) fn resume_after_deferred_native_call(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
) -> Result<VmExit, RuntimeError> {
    l.status = ThreadStatus::Yield;
    if deferred.saved_ci == 0 {
        let proto = deferred
            .caller_proto
            .ok_or_else(|| RuntimeError::new("VM: deferred root call lost its Proto"))?;
        execute_proto(l, proto, gc)
    } else {
        let result = resume_lua_thread(l, gc)?;
        if l.current_thread.is_none()
            && matches!(result, VmExit::Complete(ExecResult::Returned))
            && l.current_ci == 0
            && let Some(root_proto) = l.current_call_info().proto
        {
            l.status = ThreadStatus::Yield;
            return execute_proto(l, root_proto, gc);
        }
        Ok(result)
    }
}

fn validate_deferred_native_call(
    l: &LuaState,
    deferred: &DeferredNativeCall,
) -> Result<(), RuntimeError> {
    if l.current_ci != deferred.saved_ci + 1 {
        return Err(RuntimeError::new(
            "VM: deferred native call frame depth changed before delivery",
        ));
    }
    let ci = l.current_call_info();
    if ci.func != deferred.func_pos
        || ci.nargs != deferred.nargs as i32
        || ci.proto.is_some()
        || deferred.caller_proto != l.call_stack[deferred.saved_ci].proto
        || deferred.caller_pc != l.call_stack[deferred.saved_ci].savedpc
    {
        return Err(RuntimeError::new(
            "VM: deferred native call continuation no longer matches",
        ));
    }
    Ok(())
}

fn apply_vm_continuation(
    l: &mut LuaState,
    _gc: &mut GarbageCollector,
    continuation: &DeferredVmContinuation,
    saved_ci: usize,
    caller_pc: Option<usize>,
) -> Result<(), RuntimeError> {
    match continuation {
        DeferredVmContinuation::Call => Ok(()),
        DeferredVmContinuation::GenericFor { base, register } => {
            let first_result = l
                .stack
                .at(*base + *register + 3)
                .cloned()
                .unwrap_or(Value::Nil);
            if first_result.is_nil() {
                let caller = l.call_stack.get_mut(saved_ci).ok_or_else(|| {
                    RuntimeError::new("VM: generic-for continuation lost caller frame")
                })?;
                caller.savedpc = caller_pc.and_then(|pc| pc.checked_add(1));
            } else if let Some(destination) = l.stack.at_mut(*base + *register + 2) {
                *destination = first_result;
            }
            Ok(())
        }
        DeferredVmContinuation::TailCall => Ok(()),
    }
}

/// Next externally visible step of the Runtime-owned `table.sort` state machine.
#[derive(Debug)]
pub(crate) enum TableSortAdvance {
    Callback { function: Value, args: Vec<Value> },
    Complete,
    Error(String),
}

/// Next externally visible step of a Runtime-owned `table.foreach[i]` traversal.
#[derive(Debug)]
pub(crate) enum TableForeachAdvance {
    Callback { function: Value, args: Vec<Value> },
    Complete,
}

pub(crate) fn advance_table_foreach_state(
    gc: &GarbageCollector,
    state: &mut TableForeachState,
) -> Result<TableForeachAdvance, RuntimeError> {
    let args = match state.cursor.clone() {
        TableForeachCursor::Pairs { key } => {
            let next = gc
                .with_ref(state.table, |table| table.next(&key))
                .map_err(|error| {
                    RuntimeError::new(format!("invalid table.foreach target: {error}"))
                })?;
            let Some((next_key, value)) = next else {
                return Ok(TableForeachAdvance::Complete);
            };
            state.cursor = TableForeachCursor::Pairs {
                key: next_key.clone(),
            };
            vec![next_key, value]
        }
        TableForeachCursor::Indices { next, limit } => {
            if next > limit {
                return Ok(TableForeachAdvance::Complete);
            }
            let value = gc
                .with_ref(state.table, |table| table.get(&Value::Number(next as f64)))
                .map_err(|error| {
                    RuntimeError::new(format!("invalid table.foreachi target: {error}"))
                })?;
            state.cursor = TableForeachCursor::Indices {
                next: next.saturating_add(1),
                limit,
            };
            vec![Value::Number(next as f64), value]
        }
    };
    Ok(TableForeachAdvance::Callback {
        function: state.callback.clone(),
        args,
    })
}

/// Apply one successful `require` loader result to `package.loaded` and return
/// the exact value observed after the loader's own table writes.
pub(crate) fn finish_package_require_callback(
    gc: &mut GarbageCollector,
    loaded: GcRef<Table>,
    module_key: &Value,
    values: Vec<Value>,
) -> Result<Vec<Value>, RuntimeError> {
    if let Some(result) = values.into_iter().next()
        && !result.is_nil()
    {
        set_package_loaded_value(gc, loaded, module_key, &result)?;
    }

    let mut loaded_value = gc
        .with_ref(loaded, |table| table.get(module_key))
        .map_err(|error| RuntimeError::new(format!("invalid package.loaded table: {error}")))?;
    if loaded_value.is_nil() {
        loaded_value = Value::Boolean(true);
        set_package_loaded_value(gc, loaded, module_key, &loaded_value)?;
    }
    Ok(vec![loaded_value])
}

fn set_package_loaded_value(
    gc: &mut GarbageCollector,
    loaded: GcRef<Table>,
    module_key: &Value,
    value: &Value,
) -> Result<(), RuntimeError> {
    gc.with_mut(loaded, |table| table.set(module_key, value))
        .map_err(|error| RuntimeError::new(format!("invalid package.loaded table: {error}")))?;
    gc.write_barrier_value(loaded, module_key);
    gc.write_barrier_value(loaded, value);
    Ok(())
}

/// Next externally visible step of a Runtime-owned `string.gsub` callback loop.
#[derive(Debug)]
pub(crate) enum GsubAdvance {
    Callback { function: Value, args: Vec<Value> },
    Complete,
    Error(String),
}

pub(crate) fn advance_gsub_state(
    gc: &GarbageCollector,
    state: &mut GsubState,
    callback_result: Option<Value>,
) -> Result<GsubAdvance, RuntimeError> {
    if let Some(value) = callback_result {
        let step = state.steps.get(state.next).ok_or_else(|| {
            RuntimeError::new("string.gsub callback result lost its current match")
        })?;
        match gsub_replacement_bytes(gc, &step.matched, &value) {
            Ok(bytes) => state.output.extend_from_slice(&bytes),
            Err(message) => return Ok(GsubAdvance::Error(message)),
        }
        state.next = state.next.saturating_add(1);
    }

    loop {
        let Some(step) = state.steps.get(state.next).cloned() else {
            if !state.completed {
                state.output.extend_from_slice(&state.suffix);
                state.completed = true;
            }
            return Ok(GsubAdvance::Complete);
        };
        state.output.extend_from_slice(&step.prefix);

        match &state.replacement {
            GsubReplacement::Function(function) => {
                return Ok(GsubAdvance::Callback {
                    function: function.clone(),
                    args: step.arguments,
                });
            }
            GsubReplacement::Table { table, index_key } => {
                match resolve_gsub_table_replacement(gc, *table, index_key, &step.table_key)? {
                    GsubTableReplacement::Value(value) => {
                        match gsub_replacement_bytes(gc, &step.matched, &value) {
                            Ok(bytes) => state.output.extend_from_slice(&bytes),
                            Err(message) => return Ok(GsubAdvance::Error(message)),
                        }
                        state.next = state.next.saturating_add(1);
                    }
                    GsubTableReplacement::Callback { function, args } => {
                        return Ok(GsubAdvance::Callback { function, args });
                    }
                    GsubTableReplacement::ChainTooLong => {
                        return Ok(GsubAdvance::Error("'__index' chain too long".to_string()));
                    }
                }
            }
        }
    }
}

enum GsubTableReplacement {
    Value(Value),
    Callback { function: Value, args: Vec<Value> },
    ChainTooLong,
}

fn resolve_gsub_table_replacement(
    gc: &GarbageCollector,
    table: GcRef<Table>,
    index_key: &Value,
    key: &Value,
) -> Result<GsubTableReplacement, RuntimeError> {
    let mut current = table;
    for _ in 0..100 {
        let raw_value = gc
            .with_ref(current, |table| table.get(key))
            .map_err(|error| {
                RuntimeError::new(format!("invalid gsub replacement table: {error}"))
            })?;
        if !raw_value.is_nil() {
            return Ok(GsubTableReplacement::Value(raw_value));
        }

        let metatable = gc.with_ref(current, Table::metatable).map_err(|error| {
            RuntimeError::new(format!("invalid gsub replacement table: {error}"))
        })?;
        let Some(metatable) = metatable else {
            return Ok(GsubTableReplacement::Value(Value::Nil));
        };
        let index = gc
            .with_ref(metatable, |table| table.get(index_key))
            .map_err(|error| {
                RuntimeError::new(format!("invalid gsub replacement metatable: {error}"))
            })?;
        match index {
            Value::Table(next) => current = next,
            function @ Value::Function(_) => {
                return Ok(GsubTableReplacement::Callback {
                    function,
                    args: vec![Value::Table(current), key.clone()],
                });
            }
            _ => return Ok(GsubTableReplacement::Value(Value::Nil)),
        }
    }
    Ok(GsubTableReplacement::ChainTooLong)
}

fn gsub_replacement_bytes(
    gc: &GarbageCollector,
    matched: &[u8],
    value: &Value,
) -> Result<Vec<u8>, String> {
    match value {
        Value::Nil | Value::Boolean(false) => Ok(matched.to_vec()),
        Value::String(string) => gc
            .with_string_bytes(*string, <[u8]>::to_vec)
            .map_err(|error| format!("invalid gsub replacement string: {error}")),
        Value::Number(number) => Ok(lua_number_bytes(*number)),
        _ => Err("invalid replacement value (a string expected)".to_string()),
    }
}

fn lua_number_bytes(number: f64) -> Vec<u8> {
    if number.fract() == 0.0 && number.is_finite() {
        format!("{number:.0}").into_bytes()
    } else {
        number.to_string().into_bytes()
    }
}

enum DefaultTableSortComparison {
    Ready(bool),
    Callback(Value),
    Error(String),
}

/// Apply at most one completed comparator result, then run synchronous merge
/// work until another Lua callback, completion, or error boundary is reached.
pub(crate) fn advance_table_sort_state(
    gc: &mut GarbageCollector,
    state: &mut TableSortState,
    callback_result: Option<bool>,
) -> Result<TableSortAdvance, RuntimeError> {
    match (state.pending.take(), callback_result) {
        (Some(TableSortPending::ValidatePrimary), Some(result)) => {
            let TableSortStage::ValidateComparator { index } = state.stage.clone() else {
                return Err(RuntimeError::new(
                    "table.sort validation callback lost its stage",
                ));
            };
            if result {
                let comparator = state.comparator.clone().ok_or_else(|| {
                    RuntimeError::new("table.sort validation lost its comparator")
                })?;
                let value = state.values.get(index).cloned().ok_or_else(|| {
                    RuntimeError::new("table.sort validation index moved out of range")
                })?;
                state.pending = Some(TableSortPending::ValidateReverse);
                return Ok(TableSortAdvance::Callback {
                    function: comparator,
                    args: vec![value.clone(), value],
                });
            }
            state.stage = TableSortStage::ValidateComparator { index: index + 1 };
        }
        (Some(TableSortPending::ValidateReverse), Some(_)) => {
            return Ok(TableSortAdvance::Error(
                "invalid order function for sorting".to_string(),
            ));
        }
        (Some(TableSortPending::MergePrimary { custom }), Some(result)) => {
            if result && custom {
                let comparator = state.comparator.clone().ok_or_else(|| {
                    RuntimeError::new("table.sort reverse check lost its comparator")
                })?;
                let (first, second) = table_sort_merge_indices(state)?;
                let first_value = state.values.get(first).cloned().ok_or_else(|| {
                    RuntimeError::new("table.sort first merge index moved out of range")
                })?;
                let second_value = state.values.get(second).cloned().ok_or_else(|| {
                    RuntimeError::new("table.sort second merge index moved out of range")
                })?;
                state.pending = Some(TableSortPending::MergeReverse);
                return Ok(TableSortAdvance::Callback {
                    function: comparator,
                    args: vec![first_value, second_value],
                });
            }
            table_sort_take_merge_value(state, result)?;
        }
        (Some(TableSortPending::MergeReverse), Some(result)) => {
            if result {
                return Ok(TableSortAdvance::Error(
                    "invalid order function for sorting".to_string(),
                ));
            }
            table_sort_take_merge_value(state, true)?;
        }
        (None, None) => {}
        (Some(_), None) => {
            return Err(RuntimeError::new(
                "table.sort continuation resumed without a comparator result",
            ));
        }
        (None, Some(_)) => {
            return Err(RuntimeError::new(
                "table.sort received an unexpected comparator result",
            ));
        }
    }

    loop {
        match state.stage.clone() {
            TableSortStage::ValidateComparator { index } => {
                if state.comparator.is_none() || index >= state.values.len() {
                    state.stage = if state.values.len() < 2 {
                        TableSortStage::Complete
                    } else {
                        table_sort_merge_stage(state.values.len(), 1, 0)
                    };
                    continue;
                }
                let comparator = state
                    .comparator
                    .clone()
                    .expect("checked comparator presence above");
                let value = state.values[index].clone();
                state.pending = Some(TableSortPending::ValidatePrimary);
                return Ok(TableSortAdvance::Callback {
                    function: comparator,
                    args: vec![value.clone(), value],
                });
            }
            TableSortStage::Merge {
                width,
                left,
                middle,
                right,
                first,
                second,
                output,
            } => {
                if first < middle && second < right {
                    let lhs = state.values[second].clone();
                    let rhs = state.values[first].clone();
                    if let Some(comparator) = state.comparator.clone() {
                        state.pending = Some(TableSortPending::MergePrimary { custom: true });
                        return Ok(TableSortAdvance::Callback {
                            function: comparator,
                            args: vec![lhs, rhs],
                        });
                    }
                    match default_table_sort_comparison(gc, state, &lhs, &rhs) {
                        DefaultTableSortComparison::Ready(result) => {
                            table_sort_take_merge_value(state, result)?;
                        }
                        DefaultTableSortComparison::Callback(function) => {
                            state.pending = Some(TableSortPending::MergePrimary { custom: false });
                            return Ok(TableSortAdvance::Callback {
                                function,
                                args: vec![lhs, rhs],
                            });
                        }
                        DefaultTableSortComparison::Error(message) => {
                            return Ok(TableSortAdvance::Error(message));
                        }
                    }
                    continue;
                }

                if first < middle {
                    state.scratch[output] = state.values[first].clone();
                    state.stage = TableSortStage::Merge {
                        width,
                        left,
                        middle,
                        right,
                        first: first + 1,
                        second,
                        output: output + 1,
                    };
                    continue;
                }
                if second < right {
                    state.scratch[output] = state.values[second].clone();
                    state.stage = TableSortStage::Merge {
                        width,
                        left,
                        middle,
                        right,
                        first,
                        second: second + 1,
                        output: output + 1,
                    };
                    continue;
                }

                let span = width.saturating_mul(2);
                let next_left = left.saturating_add(span);
                if next_left < state.values.len() {
                    state.stage = table_sort_merge_stage(state.values.len(), width, next_left);
                    continue;
                }

                std::mem::swap(&mut state.values, &mut state.scratch);
                let next_width = if width > state.values.len() / 2 {
                    state.values.len()
                } else {
                    width.saturating_mul(2)
                };
                state.stage = if next_width >= state.values.len() {
                    TableSortStage::Complete
                } else {
                    table_sort_merge_stage(state.values.len(), next_width, 0)
                };
            }
            TableSortStage::Complete => return Ok(TableSortAdvance::Complete),
        }
    }
}

fn table_sort_merge_stage(size: usize, width: usize, left: usize) -> TableSortStage {
    let middle = left.saturating_add(width).min(size);
    let right = left.saturating_add(width.saturating_mul(2)).min(size);
    TableSortStage::Merge {
        width,
        left,
        middle,
        right,
        first: left,
        second: middle,
        output: left,
    }
}

fn table_sort_merge_indices(state: &TableSortState) -> Result<(usize, usize), RuntimeError> {
    match state.stage {
        TableSortStage::Merge { first, second, .. } => Ok((first, second)),
        _ => Err(RuntimeError::new(
            "table.sort comparator callback lost its merge stage",
        )),
    }
}

fn table_sort_take_merge_value(
    state: &mut TableSortState,
    take_second: bool,
) -> Result<(), RuntimeError> {
    let TableSortStage::Merge {
        width,
        left,
        middle,
        right,
        mut first,
        mut second,
        output,
    } = state.stage.clone()
    else {
        return Err(RuntimeError::new(
            "table.sort comparator result lost its merge stage",
        ));
    };
    let source = if take_second {
        let source = second;
        second += 1;
        source
    } else {
        let source = first;
        first += 1;
        source
    };
    state.scratch[output] = state.values.get(source).cloned().ok_or_else(|| {
        RuntimeError::new("table.sort comparator selected an out-of-range merge value")
    })?;
    state.stage = TableSortStage::Merge {
        width,
        left,
        middle,
        right,
        first,
        second,
        output: output + 1,
    };
    Ok(())
}

fn default_table_sort_comparison(
    gc: &GarbageCollector,
    state: &TableSortState,
    lhs: &Value,
    rhs: &Value,
) -> DefaultTableSortComparison {
    match (lhs, rhs) {
        (Value::Number(lhs), Value::Number(rhs)) => DefaultTableSortComparison::Ready(lhs < rhs),
        (Value::String(lhs), Value::String(rhs)) => match gc
            .with_string_bytes(*lhs, |lhs| {
                gc.with_string_bytes(*rhs, |rhs| compare_lua_string_bytes(lhs, rhs))
            })
            .and_then(|ordering| ordering)
        {
            Ok(ordering) => DefaultTableSortComparison::Ready(ordering == Ordering::Less),
            Err(error) => DefaultTableSortComparison::Error(format!(
                "invalid string comparison in table.sort: {error}"
            )),
        },
        _ => match common_table_sort_metamethod(gc, state, lhs, rhs) {
            Some(function) => DefaultTableSortComparison::Callback(function),
            None => DefaultTableSortComparison::Error(
                "attempt to compare values in table.sort".to_string(),
            ),
        },
    }
}

fn common_table_sort_metamethod(
    gc: &GarbageCollector,
    state: &TableSortState,
    lhs: &Value,
    rhs: &Value,
) -> Option<Value> {
    let key = state.less_key?;
    let lookup = |value: &Value| {
        let metatable = table_sort_value_metatable(gc, state, value)?;
        gc.with_ref(metatable, |table| table.get(&Value::String(key)))
            .ok()
            .filter(|value| !value.is_nil())
    };
    match (lookup(lhs), lookup(rhs)) {
        (Some(lhs), Some(rhs)) if values_equal(&lhs, &rhs) => Some(lhs),
        _ => None,
    }
}

fn table_sort_value_metatable(
    gc: &GarbageCollector,
    state: &TableSortState,
    value: &Value,
) -> Option<GcRef<Table>> {
    match value {
        Value::Table(table) => gc.with_ref(*table, Table::metatable).ok().flatten(),
        Value::Userdata(userdata) => gc
            .with_ref(*userdata, |userdata| userdata.metatable())
            .ok()
            .flatten(),
        Value::Nil => state.nil_metatable,
        Value::Boolean(_) => state.boolean_metatable,
        Value::Number(_) => state.number_metatable,
        _ => None,
    }
}

/// Commit a successfully sorted working copy atomically with respect to Lua
/// callback errors, then shade every newly published collectable edge.
pub(crate) fn commit_table_sort(
    gc: &mut GarbageCollector,
    state: &TableSortState,
) -> Result<(), RuntimeError> {
    gc.with_mut(state.table, |table| {
        for (index, value) in state.values.iter().enumerate() {
            table.set(&Value::Number((index + 1) as f64), value);
        }
    })
    .map_err(|error| RuntimeError::new(format!("invalid table.sort target: {error}")))?;
    for value in &state.values {
        gc.write_barrier_value(state.table, value);
    }
    Ok(())
}

fn invoke_runtime_native(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    operation: RuntimeNativeFunction,
) -> i32 {
    match operation {
        RuntimeNativeFunction::CollectGarbage => {
            return invoke_collectgarbage_runtime_native(l, gc);
        }
        RuntimeNativeFunction::DebugGetUpvalue => {
            return invoke_debug_upvalue_runtime_native(l, gc, false);
        }
        RuntimeNativeFunction::DebugSetUpvalue => {
            return invoke_debug_upvalue_runtime_native(l, gc, true);
        }
        RuntimeNativeFunction::ProtectedCall => {
            return invoke_protected_call_runtime_native(l, gc, ProtectedCallKind::PCall);
        }
        RuntimeNativeFunction::ProtectedXCall => {
            return invoke_protected_call_runtime_native(l, gc, ProtectedCallKind::XPCall);
        }
        RuntimeNativeFunction::ToString => {
            return invoke_tostring_runtime_native(l, gc);
        }
        RuntimeNativeFunction::Load => {
            return invoke_load_runtime_native(l, gc);
        }
        RuntimeNativeFunction::TableSort => {
            return invoke_table_sort_runtime_native(l, gc);
        }
        RuntimeNativeFunction::DoFile => {
            return invoke_dofile_runtime_native(l, gc);
        }
        RuntimeNativeFunction::TableForeach => {
            return invoke_table_foreach_runtime_native(l, gc, false);
        }
        RuntimeNativeFunction::TableForeachi => {
            return invoke_table_foreach_runtime_native(l, gc, true);
        }
        RuntimeNativeFunction::CoroutineResume | RuntimeNativeFunction::CoroutineWrapRunner => {}
    }

    let (thread_ref, args, envelope) = match operation {
        RuntimeNativeFunction::CoroutineResume => {
            let Value::Thread(thread_ref) = l.at(1).cloned().unwrap_or(Value::Nil) else {
                return publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Resume,
                    b"bad argument #1 to 'resume' (coroutine expected)",
                );
            };
            (thread_ref, native_args_from(l, 2), ResumeEnvelope::Resume)
        }
        RuntimeNativeFunction::CoroutineWrapRunner => {
            let Some(Value::Thread(thread_ref)) = current_native_upvalue(l, gc, 0) else {
                return publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Wrap,
                    b"coroutine wrapper is missing its thread",
                );
            };
            (thread_ref, native_args_from(l, 1), ResumeEnvelope::Wrap)
        }
        RuntimeNativeFunction::CollectGarbage
        | RuntimeNativeFunction::DebugGetUpvalue
        | RuntimeNativeFunction::DebugSetUpvalue
        | RuntimeNativeFunction::ProtectedCall
        | RuntimeNativeFunction::ProtectedXCall
        | RuntimeNativeFunction::ToString
        | RuntimeNativeFunction::Load
        | RuntimeNativeFunction::TableSort
        | RuntimeNativeFunction::DoFile
        | RuntimeNativeFunction::TableForeach
        | RuntimeNativeFunction::TableForeachi => unreachable!("handled above"),
    };

    let (status, target) = match gc.with_ref(thread_ref, |thread: &Thread| {
        (thread.status(), thread.state_handle())
    }) {
        Ok((status, Some(target))) => (status, target),
        _ => {
            return publish_runtime_native_error(l, gc, envelope, b"invalid coroutine");
        }
    };
    let status_error = match status {
        CoroutineStatus::Dead => Some(b"cannot resume dead coroutine".as_slice()),
        CoroutineStatus::Running => Some(b"cannot resume running coroutine".as_slice()),
        CoroutineStatus::Suspended | CoroutineStatus::Normal => None,
    };
    if let Some(message) = status_error {
        return publish_runtime_native_error(l, gc, envelope, message);
    }

    match l.publish_resume_request(thread_ref, target, args, envelope) {
        Ok(_) => 0,
        Err(error) => {
            let message = match error {
                NativeRequestPublishError::ScopeUnavailable => {
                    b"runtime-native dispatcher unavailable".as_slice()
                }
                NativeRequestPublishError::MailboxOccupied => {
                    b"runtime-native mailbox already occupied".as_slice()
                }
                NativeRequestPublishError::IdExhausted => {
                    b"runtime-native request id exhausted".as_slice()
                }
            };
            publish_runtime_native_error(l, gc, envelope, message)
        }
    }
}

fn invoke_protected_call_runtime_native(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    kind: ProtectedCallKind,
) -> i32 {
    let minimum = match kind {
        ProtectedCallKind::PCall => 1,
        ProtectedCallKind::XPCall => 2,
    };
    if l.get_top() < minimum {
        let message = match kind {
            ProtectedCallKind::PCall => {
                b"bad argument #1 to 'pcall' (function expected)".as_slice()
            }
            ProtectedCallKind::XPCall => {
                b"bad argument to 'xpcall' (function and handler expected)".as_slice()
            }
        };
        return publish_runtime_native_error(l, gc, ResumeEnvelope::ProtectedWrap, message);
    }

    let function = l.at(1).cloned().unwrap_or(Value::Nil);
    let (args, handler) = match kind {
        ProtectedCallKind::PCall => (native_args_from(l, 2), None),
        ProtectedCallKind::XPCall => (Vec::new(), Some(l.at(2).cloned().unwrap_or(Value::Nil))),
    };
    match l.publish_protected_call_request(kind, function, args, handler) {
        Ok(_) => 0,
        Err(error) => {
            let message = match error {
                NativeRequestPublishError::ScopeUnavailable => {
                    b"runtime-native dispatcher unavailable".as_slice()
                }
                NativeRequestPublishError::MailboxOccupied => {
                    b"runtime-native mailbox already occupied".as_slice()
                }
                NativeRequestPublishError::IdExhausted => {
                    b"runtime-native request id exhausted".as_slice()
                }
            };
            publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message)
        }
    }
}

fn invoke_tostring_runtime_native(l: &mut LuaState, gc: &mut GarbageCollector) -> i32 {
    if l.get_top() < 1 {
        return publish_runtime_native_error(
            l,
            gc,
            ResumeEnvelope::Wrap,
            b"bad argument #1 to 'tostring' (value expected)",
        );
    }

    let value = l.at(1).cloned().unwrap_or(Value::Nil);
    if matches!(value, Value::String(_)) {
        l.push_value(value);
        return 1;
    }

    if let Some(metatable) = value_metatable(l, gc, &value)
        && let Some(metamethod) = lookup_metamethod(l, gc, metatable, "__tostring")
        && matches!(metamethod, Value::Function(_))
    {
        return match l.publish_callback_helper_request(
            CallbackHelperKind::ToString,
            metamethod,
            vec![value],
        ) {
            Ok(_) => 0,
            Err(error) => {
                let message = match error {
                    NativeRequestPublishError::ScopeUnavailable => {
                        b"runtime-native dispatcher unavailable".as_slice()
                    }
                    NativeRequestPublishError::MailboxOccupied => {
                        b"runtime-native mailbox already occupied".as_slice()
                    }
                    NativeRequestPublishError::IdExhausted => {
                        b"runtime-native request id exhausted".as_slice()
                    }
                };
                publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message)
            }
        };
    }

    let rendered = value_to_string(gc, &value);
    gc.with_publication(|transaction| {
        let string = rooted_vm_bytes(l, transaction, rendered.as_bytes())
            .expect("tostring requires the active Runtime StringPool");
        // SAFETY: the new string is installed on the active native frame before
        // the publication root is released.
        unsafe {
            transaction.publish_string_value(string, |string| {
                l.push_value(string);
                1
            })
        }
        .expect("tostring result remains collector-registered")
    })
}

fn invoke_load_runtime_native(l: &mut LuaState, gc: &mut GarbageCollector) -> i32 {
    let reader = l.at(1).cloned().unwrap_or(Value::Nil);
    let chunk_name = l
        .at(2)
        .and_then(|value| match value {
            Value::String(name) => gc.with_string_bytes(*name, <[u8]>::to_vec).ok(),
            _ => None,
        })
        .unwrap_or_else(|| b"=(load)".to_vec());

    match reader {
        Value::String(source) => {
            let Ok(source) = gc.with_string_bytes(source, <[u8]>::to_vec) else {
                return publish_load_error(l, gc, "invalid source string");
            };
            match compile_chunk_function(l, gc, &source, &chunk_name) {
                Ok(_) => 1,
                Err(message) => publish_load_error(l, gc, &message),
            }
        }
        Value::Function(_) => match l.publish_callback_helper_request(
            CallbackHelperKind::LoadReader {
                chunk_name,
                source: Vec::new(),
                terminal_error: None,
            },
            reader,
            Vec::new(),
        ) {
            Ok(_) => 0,
            Err(error) => {
                let message = match error {
                    NativeRequestPublishError::ScopeUnavailable => {
                        "runtime-native dispatcher unavailable"
                    }
                    NativeRequestPublishError::MailboxOccupied => {
                        "runtime-native mailbox already occupied"
                    }
                    NativeRequestPublishError::IdExhausted => "runtime-native request id exhausted",
                };
                publish_load_error(l, gc, message)
            }
        },
        _ => publish_load_error(l, gc, "bad argument #1 to 'load' (function expected)"),
    }
}

/// Publish one `require` loader callback while retaining its exact
/// `package.loaded` destination for Runtime-owned completion.
pub fn request_package_require_callback(
    l: &mut LuaState,
    loader: Value,
    module_key: Value,
    loaded: GcRef<Table>,
) -> Result<(), RuntimeError> {
    let args = vec![module_key.clone()];
    l.publish_callback_helper_request(
        CallbackHelperKind::PackageRequire { loaded, module_key },
        loader,
        args,
    )
    .map(|_| ())
    .map_err(native_request_runtime_error)
}

/// Publish the first ordered `module` option callback. Runtime retains the
/// remaining options and advances them only after each preceding callback
/// succeeds.
pub fn request_package_module_option_callbacks(
    l: &mut LuaState,
    module: GcRef<Table>,
    options: Vec<Value>,
) -> Result<(), RuntimeError> {
    let Some(function) = options.first().cloned() else {
        return Ok(());
    };
    if !matches!(function, Value::Function(_)) {
        return Err(RuntimeError::new("module option must be a function"));
    }
    l.publish_callback_helper_request(
        CallbackHelperKind::PackageModuleOptions {
            state: Box::new(PackageModuleOptionsState {
                module,
                options,
                next: 1,
            }),
            terminal_error: None,
        },
        function,
        vec![Value::Table(module)],
    )
    .map(|_| ())
    .map_err(native_request_runtime_error)
}

/// Prepare and, when needed, publish callback-capable `string.gsub` work.
/// Raw table replacements may complete synchronously; the first Lua callback
/// transfers the remaining state to the Runtime scheduler.
pub fn request_gsub_callback_helper(
    l: &mut LuaState,
    gc: &GarbageCollector,
    replacement: GsubCallbackReplacement,
    steps: Vec<GsubCallbackStep>,
    suffix: Vec<u8>,
) -> Result<GsubCallbackRequestOutcome, RuntimeError> {
    let replacement = match replacement {
        GsubCallbackReplacement::Function(function) => GsubReplacement::Function(function),
        GsubCallbackReplacement::Table { table, index_key } => {
            GsubReplacement::Table { table, index_key }
        }
    };
    let steps = steps
        .into_iter()
        .map(|step| GsubStep {
            prefix: step.prefix,
            matched: step.matched,
            arguments: step.arguments,
            table_key: step.table_key,
        })
        .collect();
    let mut state = GsubState {
        replacement,
        steps,
        next: 0,
        output: Vec::new(),
        suffix,
        completed: false,
    };
    match advance_gsub_state(gc, &mut state, None)? {
        GsubAdvance::Callback { function, args } => {
            l.publish_callback_helper_request(
                CallbackHelperKind::Gsub {
                    state: Box::new(state),
                    terminal_error: None,
                },
                function,
                args,
            )
            .map_err(native_request_runtime_error)?;
            Ok(GsubCallbackRequestOutcome::Suspended)
        }
        GsubAdvance::Complete => Ok(GsubCallbackRequestOutcome::Complete {
            output: state.output,
            count: state.next,
        }),
        GsubAdvance::Error(message) => Err(RuntimeError::new(message)),
    }
}

fn invoke_dofile_runtime_native(l: &mut LuaState, gc: &mut GarbageCollector) -> i32 {
    let filename = match l.at(1) {
        Some(Value::String(filename)) => gc
            .with_string_bytes(*filename, |bytes| {
                std::str::from_utf8(bytes).ok().map(str::to_owned)
            })
            .ok()
            .flatten(),
        _ => None,
    };
    let Some(filename) = filename else {
        return publish_runtime_native_error(
            l,
            gc,
            ResumeEnvelope::Wrap,
            b"bad argument #1 to 'dofile' (string expected)",
        );
    };
    let source = match std::fs::read(&filename) {
        Ok(source) => source,
        Err(error) => {
            let message = format!("cannot open {filename}: {error}");
            return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message.as_bytes());
        }
    };
    let chunk_name = format!("@{filename}");
    let function = match compile_chunk_function(l, gc, &source, chunk_name.as_bytes()) {
        Ok(function) => Value::Function(function),
        Err(message) => {
            return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message.as_bytes());
        }
    };
    match l.publish_callback_helper_request(
        CallbackHelperKind::DoFile,
        function.clone(),
        Vec::new(),
    ) {
        Ok(_) => 0,
        Err(error) => {
            let _ = l.pop();
            let message = native_request_publication_error(error);
            publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message)
        }
    }
}

fn invoke_table_foreach_runtime_native(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    indexed: bool,
) -> i32 {
    let helper_name = if indexed {
        "table.foreachi"
    } else {
        "table.foreach"
    };
    let Value::Table(table) = l.at(1).cloned().unwrap_or(Value::Nil) else {
        let message = format!("bad argument #1 to '{helper_name}' (table expected)");
        return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message.as_bytes());
    };
    let callback = match l.at(2).cloned().unwrap_or(Value::Nil) {
        value @ Value::Function(_) => value,
        _ => {
            let message = format!("bad argument #2 to '{helper_name}' (function expected)");
            return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message.as_bytes());
        }
    };
    let cursor = if indexed {
        let n_key = interned_name_ref(l, gc, b"n");
        let limit = match gc.with_ref(table, |table| {
            n_key
                .and_then(|key| match table.get(&Value::String(key)) {
                    Value::Number(value) if value >= 0.0 => Some(value as usize),
                    _ => None,
                })
                .unwrap_or_else(|| table.length())
        }) {
            Ok(limit) => limit,
            Err(error) => {
                let message = format!("invalid {helper_name} target: {error}");
                return publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Wrap,
                    message.as_bytes(),
                );
            }
        };
        TableForeachCursor::Indices { next: 1, limit }
    } else {
        TableForeachCursor::Pairs { key: Value::Nil }
    };
    let mut state = TableForeachState {
        table,
        callback,
        cursor,
    };
    match advance_table_foreach_state(gc, &mut state) {
        Ok(TableForeachAdvance::Complete) => {
            l.push_value(Value::Nil);
            1
        }
        Ok(TableForeachAdvance::Callback { function, args }) => {
            match l.publish_callback_helper_request(
                CallbackHelperKind::TableForeach {
                    state: Box::new(state),
                },
                function,
                args,
            ) {
                Ok(_) => 0,
                Err(error) => publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Wrap,
                    native_request_publication_error(error),
                ),
            }
        }
        Err(error) => {
            publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, error.message.as_bytes())
        }
    }
}

fn publish_load_error(l: &mut LuaState, gc: &mut GarbageCollector, message: &str) -> i32 {
    l.push_value(Value::Nil);
    gc.with_publication(|transaction| {
        let message = rooted_vm_bytes(l, transaction, message.as_bytes())
            .expect("load errors require the active Runtime StringPool");
        // SAFETY: the error is installed in the active native frame before its
        // publication root is released.
        unsafe {
            transaction.publish_string_value(message, |message| {
                l.push_value(message);
                2
            })
        }
        .expect("load error remains collector-registered")
    })
}

fn invoke_table_sort_runtime_native(l: &mut LuaState, gc: &mut GarbageCollector) -> i32 {
    if l.get_top() < 1 {
        return publish_runtime_native_error(
            l,
            gc,
            ResumeEnvelope::Wrap,
            b"table.sort: missing table argument",
        );
    }
    let Value::Table(table) = l.at(1).cloned().unwrap_or(Value::Nil) else {
        return publish_runtime_native_error(
            l,
            gc,
            ResumeEnvelope::Wrap,
            b"bad argument #1 to 'table.sort' (table expected)",
        );
    };
    let comparator = if l.get_top() >= 2 {
        match l.at(2).cloned().unwrap_or(Value::Nil) {
            value @ Value::Function(_) => Some(value),
            _ => {
                return publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Wrap,
                    b"bad argument #2 to 'table.sort' (function expected)",
                );
            }
        }
    } else {
        None
    };

    let len = match gc.with_ref(table, Table::length) {
        Ok(len) => len,
        Err(error) => {
            let message = format!("invalid table.sort target: {error}");
            return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message.as_bytes());
        }
    };
    let mut values = Vec::new();
    let mut scratch = Vec::new();
    if values.try_reserve_exact(len).is_err() || scratch.try_reserve_exact(len).is_err() {
        return publish_runtime_native_error(
            l,
            gc,
            ResumeEnvelope::Wrap,
            b"not enough memory while preparing table.sort",
        );
    }
    for index in 1..=len {
        let value = match gc.with_ref(table, |table| table.get(&Value::Number(index as f64))) {
            Ok(value) => value,
            Err(error) => {
                let message = format!("invalid table.sort target: {error}");
                return publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Wrap,
                    message.as_bytes(),
                );
            }
        };
        values.push(value);
        scratch.push(Value::Nil);
    }

    let mut state = TableSortState {
        table,
        comparator,
        less_key: interned_name_ref(l, gc, b"__lt"),
        nil_metatable: l.nil_metatable,
        boolean_metatable: l.boolean_metatable,
        number_metatable: l.number_metatable,
        values,
        scratch,
        stage: TableSortStage::ValidateComparator { index: 0 },
        pending: None,
    };
    match advance_table_sort_state(gc, &mut state, None) {
        Ok(TableSortAdvance::Complete) => match commit_table_sort(gc, &state) {
            Ok(()) => 0,
            Err(error) => {
                publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, error.message.as_bytes())
            }
        },
        Ok(TableSortAdvance::Error(message)) | Err(RuntimeError { message, .. }) => {
            publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message.as_bytes())
        }
        Ok(TableSortAdvance::Callback { function, args }) => {
            match l.publish_callback_helper_request(
                CallbackHelperKind::TableSort {
                    state: Box::new(state),
                    terminal_error: None,
                },
                function,
                args,
            ) {
                Ok(_) => 0,
                Err(error) => {
                    let message = match error {
                        NativeRequestPublishError::ScopeUnavailable => {
                            b"runtime-native dispatcher unavailable".as_slice()
                        }
                        NativeRequestPublishError::MailboxOccupied => {
                            b"runtime-native mailbox already occupied".as_slice()
                        }
                        NativeRequestPublishError::IdExhausted => {
                            b"runtime-native request id exhausted".as_slice()
                        }
                    };
                    publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message)
                }
            }
        }
    }
}

fn invoke_debug_upvalue_runtime_native(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    write: bool,
) -> i32 {
    let Some(Value::Function(function_ref)) = l.at(1).cloned() else {
        l.push_nil();
        return 1;
    };
    let index = match l.at(2).cloned().unwrap_or(Value::Nil) {
        Value::Number(index) if index >= 1.0 => index as usize - 1,
        _ => {
            l.push_nil();
            return 1;
        }
    };
    let Some((proto_ref, upvalue_ref)) = gc
        .with_ref(function_ref, |function| {
            Some((function.proto()?, function.upvalue(index)?))
        })
        .ok()
        .flatten()
    else {
        l.push_nil();
        return 1;
    };
    let Some(name_ref) = gc
        .with_ref(proto_ref, |proto| proto.upvalue_name(index))
        .ok()
        .flatten()
    else {
        l.push_nil();
        return 1;
    };
    let name = Value::String(name_ref);
    let operation = if write {
        DebugUpvalueOperation::Write {
            value: l.at(3).cloned().unwrap_or(Value::Nil),
        }
    } else {
        DebugUpvalueOperation::Read
    };

    let (location, closed_value) = match gc.with_ref(upvalue_ref, |upvalue| {
        (
            upvalue.open_location(),
            upvalue
                .is_closed()
                .then(|| upvalue.get_closed_value().clone()),
        )
    }) {
        Ok(state) => state,
        Err(_) => {
            let operation = if write { b"setupvalue" } else { b"getupvalue" };
            let mut message = b"debug.".to_vec();
            message.extend_from_slice(operation);
            message.extend_from_slice(b" received an invalid Upvalue");
            return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, &message);
        }
    };

    let Some((owner, stack_index)) = location else {
        return match operation {
            DebugUpvalueOperation::Read => {
                l.push_value(name);
                l.push_value(closed_value.unwrap_or(Value::Nil));
                2
            }
            DebugUpvalueOperation::Write { value } => {
                if gc
                    .with_mut(upvalue_ref, |upvalue| upvalue.set_closed_value(value))
                    .is_err()
                {
                    return publish_runtime_native_error(
                        l,
                        gc,
                        ResumeEnvelope::Wrap,
                        b"debug.setupvalue received an invalid Upvalue",
                    );
                }
                l.push_value(name);
                1
            }
        };
    };

    if l.state_handle() == Some(owner) {
        if !l.open_upvalues.contains(&upvalue_ref) {
            let operation = if write { b"setupvalue" } else { b"getupvalue" };
            let mut message = b"debug.".to_vec();
            message.extend_from_slice(operation);
            message.extend_from_slice(b" owner state lost its open Upvalue");
            return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, &message);
        }
        return match operation {
            DebugUpvalueOperation::Read => {
                let Some(value) = l.stack.at(stack_index).cloned() else {
                    return publish_runtime_native_error(
                        l,
                        gc,
                        ResumeEnvelope::Wrap,
                        b"debug.getupvalue open stack index is out of range",
                    );
                };
                l.push_value(name);
                l.push_value(value);
                2
            }
            DebugUpvalueOperation::Write { value } => {
                let Some(slot) = l.stack.at_mut(stack_index) else {
                    return publish_runtime_native_error(
                        l,
                        gc,
                        ResumeEnvelope::Wrap,
                        b"debug.setupvalue open stack index is out of range",
                    );
                };
                *slot = value;
                l.push_value(name);
                1
            }
        };
    }

    match l.publish_debug_upvalue_request(upvalue_ref, owner, stack_index, name, operation) {
        Ok(_) => 0,
        Err(error) => {
            let message = match error {
                NativeRequestPublishError::ScopeUnavailable => {
                    b"runtime-native dispatcher unavailable".as_slice()
                }
                NativeRequestPublishError::MailboxOccupied => {
                    b"runtime-native mailbox already occupied".as_slice()
                }
                NativeRequestPublishError::IdExhausted => {
                    b"runtime-native request id exhausted".as_slice()
                }
            };
            publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message)
        }
    }
}

fn invoke_collectgarbage_runtime_native(l: &mut LuaState, gc: &mut GarbageCollector) -> i32 {
    let option = match l.at(1) {
        Some(Value::String(value)) => gc.with_string_bytes(*value, <[u8]>::to_vec).ok(),
        _ => None,
    };
    match option.as_deref().unwrap_or(b"collect") {
        b"count" => {
            l.push_number(gc.total_memory() as f64 / 1024.0);
            1
        }
        b"collect" => publish_full_collection_request(l, gc, FullCollectionResult::Collect),
        b"step" => {
            let size = match collect_garbage_control_argument(l, gc) {
                Ok(size) => size,
                Err(message) => {
                    return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message);
                }
            };
            publish_full_collection_request(l, gc, FullCollectionResult::Step { size })
        }
        b"stop" => {
            gc.stop_automatic();
            l.push_number(0.0);
            1
        }
        b"restart" => {
            gc.restart_automatic();
            l.push_number(0.0);
            1
        }
        b"setpause" => {
            let value = match collect_garbage_control_argument(l, gc) {
                Ok(value) => value,
                Err(message) => {
                    return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message);
                }
            };
            l.push_number(gc.set_pause(value) as f64);
            1
        }
        b"setstepmul" => {
            let value = match collect_garbage_control_argument(l, gc) {
                Ok(value) => value,
                Err(message) => {
                    return publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message);
                }
            };
            l.push_number(gc.set_step_multiplier(value) as f64);
            1
        }
        _ => publish_runtime_native_error(
            l,
            gc,
            ResumeEnvelope::Wrap,
            b"bad argument #1 to 'collectgarbage' (invalid option)",
        ),
    }
}

fn publish_full_collection_request(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    result: FullCollectionResult,
) -> i32 {
    match l.publish_full_collection_request(result) {
        Ok(_) => 0,
        Err(error) => {
            let message = match error {
                NativeRequestPublishError::ScopeUnavailable => {
                    b"runtime-native dispatcher unavailable".as_slice()
                }
                NativeRequestPublishError::MailboxOccupied => {
                    b"runtime-native mailbox already occupied".as_slice()
                }
                NativeRequestPublishError::IdExhausted => {
                    b"runtime-native request id exhausted".as_slice()
                }
            };
            publish_runtime_native_error(l, gc, ResumeEnvelope::Wrap, message)
        }
    }
}

fn collect_garbage_control_argument(
    l: &LuaState,
    gc: &GarbageCollector,
) -> Result<i32, &'static [u8]> {
    let value = match l.at(2) {
        None | Some(Value::Nil) => return Ok(0),
        Some(Value::Number(value)) => *value,
        Some(Value::String(value)) => gc
            .with_string_bytes(*value, <[u8]>::to_vec)
            .ok()
            .and_then(|bytes| parse_lua_number_bytes(&bytes))
            .ok_or(b"bad argument #2 to 'collectgarbage' (number expected)".as_slice())?,
        Some(_) => {
            return Err(b"bad argument #2 to 'collectgarbage' (number expected)");
        }
    };
    if !value.is_finite() {
        return Err(b"bad argument #2 to 'collectgarbage' (finite number expected)");
    }
    let truncated = value.trunc();
    if truncated < i32::MIN as f64 || truncated > i32::MAX as f64 {
        return Err(b"bad argument #2 to 'collectgarbage' (number out of range)");
    }
    Ok(truncated as i32)
}

fn publish_runtime_native_error(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    envelope: ResumeEnvelope,
    message: &[u8],
) -> i32 {
    gc.with_publication(|transaction| {
        let error = rooted_vm_bytes(l, transaction, message)
            .expect("Runtime-native errors require the active Runtime StringPool");
        // SAFETY: every branch installs the error string on the active stack
        // before its publication root is released.
        unsafe {
            transaction.publish_string_value(error, |error| match envelope {
                ResumeEnvelope::Resume => {
                    l.push_boolean(false);
                    l.push_value(error);
                    2
                }
                ResumeEnvelope::Wrap => {
                    l.push_value(error);
                    -1
                }
                ResumeEnvelope::ProtectedResume => {
                    l.push_boolean(true);
                    l.push_boolean(false);
                    l.push_value(error);
                    3
                }
                ResumeEnvelope::ProtectedWrap => {
                    l.push_boolean(false);
                    l.push_value(error);
                    2
                }
            })
        }
        .expect("new Runtime-native error string remains registered")
    })
}

fn native_request_publication_error(error: NativeRequestPublishError) -> &'static [u8] {
    match error {
        NativeRequestPublishError::ScopeUnavailable => b"runtime-native dispatcher unavailable",
        NativeRequestPublishError::MailboxOccupied => b"runtime-native mailbox already occupied",
        NativeRequestPublishError::IdExhausted => b"runtime-native request id exhausted",
    }
}

fn native_request_runtime_error(error: NativeRequestPublishError) -> RuntimeError {
    RuntimeError::new(String::from_utf8_lossy(native_request_publication_error(error)).into_owned())
}

fn native_args_from(l: &LuaState, first: i32) -> Vec<Value> {
    let top = l.get_top();
    if top < first {
        return Vec::new();
    }
    (first..=top)
        .map(|index| l.at(index).cloned().unwrap_or(Value::Nil))
        .collect()
}

fn current_native_upvalue(l: &LuaState, gc: &GarbageCollector, index: usize) -> Option<Value> {
    let func_idx = l.current_call_info().func;
    let Value::Function(func_ref) = l.stack.at(func_idx).cloned()? else {
        return None;
    };
    let upvalue_ref = gc
        .with_ref(func_ref, |function| function.upvalue(index))
        .ok()
        .flatten()?;
    gc.with_ref(upvalue_ref, |upvalue| upvalue.get_closed_value().clone())
        .ok()
}

fn proto_instruction(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
) -> Result<Option<lua_core::proto::Instruction>, RuntimeError> {
    gc.with_ref(proto, |proto| proto.code().get(pc).copied())
        .map_err(invalid_proto_error)
}

fn proto_constant(
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    index: usize,
) -> Result<Value, RuntimeError> {
    gc.with_ref(proto, |proto| {
        proto.constants().get(index).cloned().unwrap_or(Value::Nil)
    })
    .map_err(invalid_proto_error)
}

fn proto_max_stack_size(gc: &GarbageCollector, proto: GcRef<Proto>) -> Result<usize, RuntimeError> {
    gc.with_ref(proto, |proto| proto.max_stack_size() as usize)
        .map_err(invalid_proto_error)
}

/// 获取 RK 操作数——寄存器或常量
fn get_rk(
    l: &LuaState,
    gc: &GarbageCollector,
    proto: GcRef<Proto>,
    base: usize,
    rk: i32,
) -> Result<Value, RuntimeError> {
    if opcode::is_k(rk) {
        let idx = opcode::index_k(rk) as usize;
        proto_constant(gc, proto, idx)
    } else {
        let reg = rk as usize;
        Ok(l.stack.at(base + reg).cloned().unwrap_or(Value::Nil))
    }
}

/// 获取上值
enum UpvalueRead {
    Ready(Value),
    Remote {
        upvalue: GcRef<Upvalue>,
        owner: lua_core::state_handle::StateHandle,
        stack_index: usize,
    },
}

fn get_upvalue(
    l: &LuaState,
    gc: &GarbageCollector,
    upvalue_idx: usize,
) -> Result<UpvalueRead, RuntimeError> {
    let uv_ref = current_lua_upvalue(l, gc, upvalue_idx)
        .ok_or_else(|| RuntimeError::new("VM: GETUPVAL invalid upvalue index"))?;

    let (location, closed) = gc
        .with_ref(uv_ref, |upvalue| {
            (
                upvalue.open_location(),
                upvalue
                    .is_closed()
                    .then(|| upvalue.get_closed_value().clone()),
            )
        })
        .map_err(|error| RuntimeError::new(format!("VM: GETUPVAL invalid Upvalue: {error}")))?;
    if let Some((owner, stack_index)) = location {
        if l.state_handle() == Some(owner) {
            if !l.open_upvalues.contains(&uv_ref) {
                return Err(RuntimeError::new(
                    "VM: GETUPVAL owner state lost its open Upvalue",
                ));
            }
            let value = l.stack.at(stack_index).cloned().ok_or_else(|| {
                RuntimeError::new("VM: GETUPVAL open stack index is out of range")
            })?;
            Ok(UpvalueRead::Ready(value))
        } else {
            Ok(UpvalueRead::Remote {
                upvalue: uv_ref,
                owner,
                stack_index,
            })
        }
    } else {
        Ok(UpvalueRead::Ready(closed.unwrap_or(Value::Nil)))
    }
}

/// 设置上值
fn set_upvalue(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    upvalue_idx: usize,
    val: &Value,
) -> Result<Option<(GcRef<Upvalue>, lua_core::state_handle::StateHandle, usize)>, RuntimeError> {
    let uv_ref = current_lua_upvalue(l, gc, upvalue_idx)
        .ok_or_else(|| RuntimeError::new("VM: SETUPVAL invalid upvalue index"))?;

    let location = gc
        .with_ref(uv_ref, Upvalue::open_location)
        .map_err(|error| RuntimeError::new(format!("VM: SETUPVAL invalid Upvalue: {error}")))?;
    if let Some((owner, stack_index)) = location {
        if l.state_handle() == Some(owner) {
            if !l.open_upvalues.contains(&uv_ref) {
                return Err(RuntimeError::new(
                    "VM: SETUPVAL owner state lost its open Upvalue",
                ));
            }
            let slot = l.stack.at_mut(stack_index).ok_or_else(|| {
                RuntimeError::new("VM: SETUPVAL open stack index is out of range")
            })?;
            *slot = val.clone();
            Ok(None)
        } else {
            Ok(Some((uv_ref, owner, stack_index)))
        }
    } else {
        gc.with_mut(uv_ref, |upvalue| upvalue.set_closed_value(val.clone()))
            .map_err(|error| RuntimeError::new(format!("VM: SETUPVAL invalid Upvalue: {error}")))?;
        Ok(None)
    }
}

fn current_lua_function_ref(l: &LuaState) -> Option<GcRef<Function>> {
    if l.current_ci == 0 {
        return None;
    }
    let ci = l.current_call_info();
    if ci.func == ci.base {
        return None;
    }
    let func_idx = ci.func;
    match l.stack.at(func_idx) {
        Some(Value::Function(func_ref)) => Some(*func_ref),
        _ => None,
    }
}

fn current_lua_proto(l: &LuaState, gc: &GarbageCollector) -> Option<GcRef<Proto>> {
    current_lua_function_ref(l)
        .and_then(|function| gc.with_ref(function, Function::proto).ok().flatten())
}

fn current_lua_upvalue(
    l: &LuaState,
    gc: &GarbageCollector,
    index: usize,
) -> Option<GcRef<Upvalue>> {
    current_lua_function_ref(l).and_then(|function| {
        gc.with_ref(function, |function| function.upvalue(index))
            .ok()
            .flatten()
    })
}

fn current_env(l: &LuaState, gc: &GarbageCollector) -> Option<GcRef<Table>> {
    match current_lua_function_ref(l) {
        Some(function) => gc
            .with_ref(function, Function::env)
            .ok()
            .flatten()
            .or(l.global_table),
        None => l.chunk_env.or(l.thread_env).or(l.global_table),
    }
}

/// 全局变量读取
fn get_global_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    name: &Value,
    destination: usize,
    proto: GcRef<Proto>,
    pc: usize,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    if let Some(env_table) = current_env(l, gc) {
        get_table_into(
            l,
            gc,
            &Value::Table(env_table),
            name,
            destination,
            VmMetamethodContinuation::StoreResult {
                proto,
                pc,
                destination,
            },
        )
    } else {
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = Value::Nil;
        }
        Ok(VmMetamethodOutcome::Ready(()))
    }
}

/// 全局变量写入
fn set_global(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    name: &Value,
    val: &Value,
    proto: GcRef<Proto>,
    pc: usize,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    if let Some(env_table) = current_env(l, gc) {
        return set_table_value(
            l,
            gc,
            &Value::Table(env_table),
            name,
            val,
            VmMetamethodContinuation::CompleteSet { proto, pc },
        );
    }
    Ok(VmMetamethodOutcome::Ready(()))
}

/// 表取值（含元方法回退）
fn get_table_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table: &Value,
    key: &Value,
    destination: usize,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    let mut current = table.clone();
    for _ in 0..100 {
        match &current {
            Value::Table(t) => {
                let Ok((result, metatable)) =
                    gc.with_ref(*t, |table| (table.get(key), table.metatable()))
                else {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = Value::Nil;
                    }
                    return Ok(VmMetamethodOutcome::Ready(()));
                };
                if !result.is_nil() {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = result;
                    }
                    return Ok(VmMetamethodOutcome::Ready(()));
                }

                let Some(index_metamethod) =
                    metatable.and_then(|mt| lookup_metamethod(l, gc, mt, "__index"))
                else {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = Value::Nil;
                    }
                    return Ok(VmMetamethodOutcome::Ready(()));
                };

                match index_metamethod {
                    Value::Function(_) => {
                        return call_metamethod_into(
                            l,
                            index_metamethod,
                            vec![current.clone(), key.clone()],
                            continuation,
                        );
                    }
                    Value::Table(_) => current = index_metamethod,
                    _ => {
                        if let Some(slot) = l.stack.at_mut(destination) {
                            *slot = Value::Nil;
                        }
                        return Ok(VmMetamethodOutcome::Ready(()));
                    }
                }
            }
            Value::String(_) => {
                let result = get_string_library_member(l, gc, key);
                if let Some(slot) = l.stack.at_mut(destination) {
                    *slot = result;
                }
                return Ok(VmMetamethodOutcome::Ready(()));
            }
            _ => {
                let Some(index_metamethod) = value_metatable(l, gc, &current)
                    .and_then(|mt| lookup_metamethod(l, gc, mt, "__index"))
                else {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = Value::Nil;
                    }
                    return Ok(VmMetamethodOutcome::Ready(()));
                };
                match index_metamethod {
                    Value::Function(_) => {
                        return call_metamethod_into(
                            l,
                            index_metamethod,
                            vec![current.clone(), key.clone()],
                            continuation,
                        );
                    }
                    Value::Table(_) => current = index_metamethod,
                    _ => {
                        if let Some(slot) = l.stack.at_mut(destination) {
                            *slot = Value::Nil;
                        }
                        return Ok(VmMetamethodOutcome::Ready(()));
                    }
                }
            }
        }
    }
    Err(RuntimeError::new("'__index' chain too long"))
}

fn get_string_library_member(l: &LuaState, gc: &GarbageCollector, key: &Value) -> Value {
    let Some(global_table) = l.global_table else {
        return Value::Nil;
    };
    let Some(string_name) = interned_name_ref(l, gc, b"string") else {
        return Value::Nil;
    };
    let string_table = gc
        .with_ref(global_table, |global| {
            global.get(&Value::String(string_name))
        })
        .ok();
    let Some(Value::Table(string_table)) = string_table else {
        return Value::Nil;
    };
    gc.with_ref(string_table, |table| table.get(key))
        .unwrap_or(Value::Nil)
}

/// 表赋值（含元方法回退）
fn set_table_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    table: &Value,
    key: &Value,
    value: &Value,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    let mut current = table.clone();
    for _ in 0..100 {
        match &current {
            Value::Table(t) => {
                if key.is_nil() {
                    return Err(RuntimeError::new("table index is nil"));
                }
                if let Value::Number(n) = key
                    && n.is_nan()
                {
                    return Err(RuntimeError::new("table index is NaN"));
                }

                let (has_raw_key, metatable) = gc
                    .with_ref(*t, |table| (table.has(key), table.metatable()))
                    .map_err(|error| {
                        RuntimeError::new(format!("VM: invalid table mutation: {error}"))
                    })?;
                if !has_raw_key {
                    if let Some(newindex_metamethod) =
                        metatable.and_then(|mt| lookup_metamethod(l, gc, mt, "__newindex"))
                    {
                        match newindex_metamethod {
                            Value::Function(_) => {
                                return call_metamethod_into(
                                    l,
                                    newindex_metamethod,
                                    vec![current, key.clone(), value.clone()],
                                    continuation,
                                );
                            }
                            Value::Table(_) => {
                                current = newindex_metamethod;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }

                gc.with_mut(*t, |table| table.set(key, value))
                    .map_err(|error| {
                        RuntimeError::new(format!("VM: invalid table mutation: {error}"))
                    })?;
                return Ok(VmMetamethodOutcome::Ready(()));
            }
            _ => {
                if let Some(newindex_metamethod) = value_metatable(l, gc, &current)
                    .and_then(|mt| lookup_metamethod(l, gc, mt, "__newindex"))
                {
                    match newindex_metamethod {
                        Value::Function(_) => {
                            return call_metamethod_into(
                                l,
                                newindex_metamethod,
                                vec![current, key.clone(), value.clone()],
                                continuation,
                            );
                        }
                        Value::Table(_) => {
                            current = newindex_metamethod;
                            continue;
                        }
                        _ => {}
                    }
                }
                return Err(RuntimeError::new(format!(
                    "attempt to index a {} value",
                    value_type_name(&current)
                )));
            }
        }
    }
    Err(RuntimeError::new("'__newindex' chain too long"))
}

/// 算术运算
fn exec_arith_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    op: OpCode,
    lhs: &Value,
    rhs: &Value,
    destination: usize,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    let metamethod_name = match op {
        OpCode::ADD => "__add",
        OpCode::SUB => "__sub",
        OpCode::MUL => "__mul",
        OpCode::DIV => "__div",
        OpCode::MOD => "__mod",
        OpCode::POW => "__pow",
        _ => return Err(RuntimeError::new("unknown arithmetic operation")),
    };

    if let (Some(a), Some(b)) = (to_arith_number(gc, lhs), to_arith_number(gc, rhs)) {
        let result = match op {
            OpCode::ADD => a + b,
            OpCode::SUB => a - b,
            OpCode::MUL => a * b,
            OpCode::DIV => {
                if b == 0.0 {
                    return Err(RuntimeError::new("attempt to divide by zero"));
                }
                a / b
            }
            OpCode::MOD => {
                if b == 0.0 {
                    return Err(RuntimeError::new("attempt to modulo by zero"));
                }
                a - (a / b).floor() * b
            }
            OpCode::POW => a.powf(b),
            _ => unreachable!(),
        };
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = Value::Number(result);
        }
        return Ok(VmMetamethodOutcome::Ready(()));
    }

    if let Some(metamethod) = find_metamethod(l, gc, lhs, rhs, metamethod_name) {
        call_metamethod_into(l, metamethod, vec![lhs.clone(), rhs.clone()], continuation)
    } else {
        Err(RuntimeError::new(
            "attempt to perform arithmetic on a non-number value",
        ))
    }
}

/// 取负
fn exec_unm_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    val: &Value,
    destination: usize,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    if let Some(n) = to_arith_number(gc, val) {
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = Value::Number(-n);
        }
        return Ok(VmMetamethodOutcome::Ready(()));
    }
    if let Some(metamethod) = find_metamethod(l, gc, val, val, "__unm") {
        call_metamethod_into(l, metamethod, vec![val.clone(), val.clone()], continuation)
    } else {
        Err(RuntimeError::new(
            "attempt to perform arithmetic on a non-number value",
        ))
    }
}

fn exec_concat_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    lhs: &Value,
    rhs: &Value,
    destination: usize,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    if let (Some(left), Some(right)) = (to_concat_bytes(gc, lhs), to_concat_bytes(gc, rhs)) {
        let len = left
            .len()
            .checked_add(right.len())
            .ok_or_else(|| RuntimeError::new("string length overflow"))?;
        if len > MAX_STRING_LENGTH {
            return Err(RuntimeError::new("string length overflow"));
        }
        let mut bytes = Vec::with_capacity(len);
        bytes.extend_from_slice(&left);
        bytes.extend_from_slice(&right);
        let pool = l.active_string_pool_ptr();
        gc.with_publication(|transaction| {
            let pool = pool.ok_or(GcRefValidationError::StringPoolUnavailable)?;
            // SAFETY: the Runtime owns the pool and VM execution has
            // exclusive access to it for this state turn.
            let string = transaction.intern_bytes(unsafe { &mut *pool }, &bytes)?;
            // SAFETY: the callback installs the result in the destination
            // VM register before releasing its temporary root.
            unsafe {
                transaction.publish_string_value(string, |value| {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = value;
                    }
                })
            }
        })
        .map_err(|error| {
            RuntimeError::new(format!("VM: could not publish concat string: {error}"))
        })?;
        return Ok(VmMetamethodOutcome::Ready(()));
    }
    if let Some(metamethod) = find_metamethod(l, gc, lhs, rhs, "__concat") {
        call_metamethod_into(l, metamethod, vec![lhs.clone(), rhs.clone()], continuation)
    } else {
        Err(RuntimeError::new(
            "attempt to concatenate a non-string value",
        ))
    }
}

/// 取长度（字符串和表走原始长度，其他值回退到 `__len`）。
fn exec_len_into(
    l: &mut LuaState,
    gc: &GarbageCollector,
    val: &Value,
    destination: usize,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    let raw_length = match val {
        Value::String(string) => Some(
            gc.with_ref(*string, |string| Value::Number(string.len() as f64))
                .map_err(|error| RuntimeError::new(format!("invalid string length: {error}")))?,
        ),
        Value::Table(table) => Some(
            gc.with_ref(*table, |table| Value::Number(table.length() as f64))
                .map_err(|error| RuntimeError::new(format!("invalid table length: {error}")))?,
        ),
        _ => None,
    };
    if let Some(length) = raw_length {
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = length;
        }
        return Ok(VmMetamethodOutcome::Ready(()));
    }
    if let Some(metamethod) = find_metamethod(l, gc, val, val, "__len") {
        call_metamethod_into(l, metamethod, vec![val.clone(), val.clone()], continuation)
    } else {
        Err(RuntimeError::new(format!(
            "attempt to get length of a {} value",
            value_type_name(val)
        )))
    }
}

/// 比较：小于
fn exec_lt(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    lhs: &Value,
    rhs: &Value,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<bool>, RuntimeError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(VmMetamethodOutcome::Ready(a < b)),
        (Value::String(a_ref), Value::String(b_ref)) => gc
            .with_string_bytes(*a_ref, |left| {
                gc.with_string_bytes(*b_ref, |right| compare_lua_string_bytes(left, right))
            })
            .and_then(|ordering| ordering)
            .map(|ordering| VmMetamethodOutcome::Ready(ordering == Ordering::Less))
            .map_err(|error| RuntimeError::new(format!("invalid string comparison: {error}"))),
        _ => {
            if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__lt") {
                call_metamethod_bool(l, metamethod, vec![lhs.clone(), rhs.clone()], continuation)
            } else {
                Err(RuntimeError::new(
                    "attempt to compare non-comparable values",
                ))
            }
        }
    }
}

/// 比较：小于等于
fn exec_le(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    lhs: &Value,
    rhs: &Value,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<bool>, RuntimeError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(VmMetamethodOutcome::Ready(a <= b)),
        (Value::String(a_ref), Value::String(b_ref)) => gc
            .with_string_bytes(*a_ref, |left| {
                gc.with_string_bytes(*b_ref, |right| compare_lua_string_bytes(left, right))
            })
            .and_then(|ordering| ordering)
            .map(|ordering| VmMetamethodOutcome::Ready(ordering != Ordering::Greater))
            .map_err(|error| RuntimeError::new(format!("invalid string comparison: {error}"))),
        _ => {
            if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__le") {
                return call_metamethod_bool(
                    l,
                    metamethod,
                    vec![lhs.clone(), rhs.clone()],
                    continuation,
                );
            }
            if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__lt") {
                let continuation = match continuation {
                    VmMetamethodContinuation::Compare {
                        proto,
                        pc,
                        expected,
                        ..
                    } => VmMetamethodContinuation::Compare {
                        proto,
                        pc,
                        expected,
                        invert_result: true,
                    },
                    _ => {
                        return Err(RuntimeError::new(
                            "VM: <= fallback lost its comparison continuation",
                        ));
                    }
                };
                return call_metamethod_bool(
                    l,
                    metamethod,
                    vec![rhs.clone(), lhs.clone()],
                    continuation,
                );
            }
            Err(RuntimeError::new(
                "attempt to compare non-comparable values",
            ))
        }
    }
}

/// Compares Lua strings like the fixed C++ oracle's `luaStringCompare`.
///
/// The oracle calls `strcoll` separately for each NUL-delimited segment and
/// then applies its explicit remaining-length rules. Under the default C
/// locale, `strcoll` is unsigned-byte lexicographic ordering, which is what
/// `segment.cmp` implements here. Locale-aware collation remains tracked by
/// NOTE-006; this comparator deliberately does not use `ByteString::Ord`.
pub fn compare_lua_string_bytes(mut left: &[u8], mut right: &[u8]) -> Ordering {
    loop {
        let left_segment_len = left
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(left.len());
        let right_segment_len = right
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(right.len());
        let segment_order = left[..left_segment_len].cmp(&right[..right_segment_len]);
        if segment_order != Ordering::Equal {
            return segment_order;
        }

        // Equal segments necessarily have the same length. Preserve the
        // oracle's tail checks exactly, including the ordering of a terminal
        // string against the same bytes followed by an embedded NUL.
        let segment_len = left_segment_len;
        if segment_len == right.len() {
            return if segment_len == left.len() {
                Ordering::Equal
            } else {
                Ordering::Greater
            };
        }
        if segment_len == left.len() {
            return Ordering::Less;
        }

        let next = segment_len + 1;
        left = &left[next..];
        right = &right[next..];
    }
}

fn call_metamethod_bool(
    l: &mut LuaState,
    metamethod: Value,
    args: Vec<Value>,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<bool>, RuntimeError> {
    let request_id = publish_vm_metamethod_call(l, metamethod, args, continuation)?;
    Ok(VmMetamethodOutcome::Suspended(request_id))
}

fn call_metamethod_into(
    l: &mut LuaState,
    metamethod: Value,
    args: Vec<Value>,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<()>, RuntimeError> {
    let request_id = publish_vm_metamethod_call(l, metamethod, args, continuation)?;
    Ok(VmMetamethodOutcome::Suspended(request_id))
}

fn exec_eq(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    lhs: &Value,
    rhs: &Value,
    continuation: VmMetamethodContinuation,
) -> Result<VmMetamethodOutcome<bool>, RuntimeError> {
    if values_equal(lhs, rhs) {
        return Ok(VmMetamethodOutcome::Ready(true));
    }
    if std::mem::discriminant(lhs) != std::mem::discriminant(rhs) {
        return Ok(VmMetamethodOutcome::Ready(false));
    }
    if !matches!(lhs, Value::Table(_) | Value::Userdata(_)) {
        return Ok(VmMetamethodOutcome::Ready(false));
    }
    if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__eq") {
        call_metamethod_bool(l, metamethod, vec![lhs.clone(), rhs.clone()], continuation)
    } else {
        Ok(VmMetamethodOutcome::Ready(false))
    }
}

fn find_metamethod(
    l: &LuaState,
    gc: &GarbageCollector,
    lhs: &Value,
    rhs: &Value,
    name: &str,
) -> Option<Value> {
    value_metatable(l, gc, lhs)
        .and_then(|metatable| lookup_metamethod(l, gc, metatable, name))
        .or_else(|| {
            value_metatable(l, gc, rhs)
                .and_then(|metatable| lookup_metamethod(l, gc, metatable, name))
        })
}

fn find_common_metamethod(
    l: &LuaState,
    gc: &GarbageCollector,
    lhs: &Value,
    rhs: &Value,
    name: &str,
) -> Option<Value> {
    let lhs_method =
        value_metatable(l, gc, lhs).and_then(|metatable| lookup_metamethod(l, gc, metatable, name));
    let rhs_method =
        value_metatable(l, gc, rhs).and_then(|metatable| lookup_metamethod(l, gc, metatable, name));
    match (lhs_method, rhs_method) {
        (Some(lhs_method), Some(rhs_method)) if values_equal(&lhs_method, &rhs_method) => {
            Some(lhs_method)
        }
        _ => None,
    }
}

fn value_metatable(l: &LuaState, gc: &GarbageCollector, value: &Value) -> Option<GcRef<Table>> {
    match value {
        Value::Table(table_ref) => gc.with_ref(*table_ref, Table::metatable).ok().flatten(),
        Value::Userdata(userdata_ref) => gc
            .with_ref(*userdata_ref, |userdata| userdata.metatable())
            .ok()
            .flatten(),
        Value::Nil => l.nil_metatable,
        Value::Boolean(_) => l.boolean_metatable,
        Value::Number(_) => l.number_metatable,
        _ => None,
    }
}

fn interned_name_ref(l: &LuaState, gc: &GarbageCollector, name: &[u8]) -> Option<GcRef<GcString>> {
    let string_pool = l.active_string_pool_ptr()?;
    // SAFETY: Runtime installs the pool only for this dynamic state turn.
    let candidate = unsafe { &*string_pool }.find_bytes(name)?;
    gc.with_ref(candidate, |_| candidate).ok()
}

fn lookup_metamethod(
    l: &LuaState,
    gc: &GarbageCollector,
    metatable: GcRef<Table>,
    name: &str,
) -> Option<Value> {
    let key = interned_name_ref(l, gc, name.as_bytes())?;
    gc.with_ref(metatable, |table| table.get(&Value::String(key)))
        .ok()
        .filter(|value| !value.is_nil())
}

fn to_arith_number(gc: &GarbageCollector, value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => Some(*n),
        Value::String(s) => gc
            .with_string_bytes(*s, parse_lua_number_bytes)
            .ok()
            .flatten(),
        _ => None,
    }
}

fn parse_lua_number_bytes(mut bytes: &[u8]) -> Option<f64> {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    std::str::from_utf8(bytes).ok()?.parse::<f64>().ok()
}

fn to_concat_bytes(gc: &GarbageCollector, value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::String(s) => gc.with_string_bytes(*s, <[u8]>::to_vec).ok(),
        Value::Number(n) => {
            let text = if n.fract() == 0.0 && n.is_finite() {
                format!("{n:.0}")
            } else {
                n.to_string()
            };
            Some(text.into_bytes())
        }
        _ => None,
    }
}

/// 值相等
fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    lhs == rhs
}

/// 提取数值（含字符串强制转换）
fn as_number(gc: &GarbageCollector, val: &Value) -> f64 {
    match val {
        Value::Number(n) => *n,
        Value::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::String(s) => gc
            .with_string_bytes(*s, parse_lua_number_bytes)
            .ok()
            .flatten()
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

/// 值转字符串（调试和 CONCAT 使用）
fn value_to_string(gc: &GarbageCollector, val: &Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => {
            // Format Lua-style: integer if whole number
            if n.fract() == 0.0 && n.is_finite() {
                format!("{:.0}", n)
            } else {
                n.to_string()
            }
        }
        Value::String(s) => gc
            .with_string_bytes(*s, |bytes| String::from_utf8_lossy(bytes).into_owned())
            .unwrap_or_default(),
        Value::Table(t) => format!("table: {:p}", t.as_ptr()),
        Value::Function(f) => format!("function: {:p}", f.as_ptr()),
        Value::Userdata(u) => format!("userdata: {:p}", u.as_ptr()),
        Value::Thread(t) => format!("thread: {:p}", t.as_ptr()),
        Value::LightUserdata(p) => format!("lightuserdata: {:p}", p.as_ptr()),
    }
}

// ═══════════════════════════════════════════════════════════════════
// RuntimeError
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub message: String,
    error_value: Option<Value>,
    native_request_suspend: bool,
}

fn invalid_proto_error(error: lua_core::gc::collector::GcRefValidationError) -> RuntimeError {
    RuntimeError::new(format!("VM: invalid Proto handle: {error}"))
}

impl RuntimeError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            error_value: None,
            native_request_suspend: false,
        }
    }

    pub fn with_value(gc: &GarbageCollector, value: Value) -> Self {
        Self {
            message: value_to_string(gc, &value),
            error_value: Some(value),
            native_request_suspend: false,
        }
    }

    pub fn error_value(&self) -> Option<Value> {
        self.error_value.clone()
    }

    fn native_request_suspend() -> Self {
        Self {
            message: "Runtime-native request suspended protected helper".to_string(),
            error_value: None,
            native_request_suspend: true,
        }
    }

    pub fn is_native_request_suspend(&self) -> bool {
        self.native_request_suspend
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RuntimeError {}

#[cfg(test)]
mod byte_string_tests {
    use super::*;
    use lua_core::string_pool::StringPool;

    unsafe extern "C" fn return_fresh_string(state: *mut std::ffi::c_void) -> i32 {
        // SAFETY: the VM passes its live LuaState to test C functions.
        let state = unsafe { &mut *state.cast::<LuaState>() };
        let Some(gc) = state.active_gc_ptr() else {
            return 0;
        };
        let Some(string_pool) = state.active_string_pool_ptr() else {
            return 0;
        };
        // SAFETY: the collector pointer is installed for this call.
        let gc = unsafe { &mut *gc };
        // SAFETY: the StringPool pointer is installed for this call.
        let string_pool = unsafe { &mut *string_pool };
        let string = string_pool.intern_bytes(gc, b"fresh result");
        state.push_value(Value::String(string));
        1
    }

    fn byte_value(gc: &mut GarbageCollector, string_pool: &mut StringPool, bytes: &[u8]) -> Value {
        Value::String(string_pool.intern_bytes(gc, bytes))
    }

    #[test]
    fn raw_ff_is_not_equal_to_utf8_encoding_of_y_diaeresis() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let raw_ff = byte_value(&mut gc, &mut pool, &[0xff]);
        let utf8_y_diaeresis = byte_value(&mut gc, &mut pool, &[0xc3, 0xbf]);

        assert!(!values_equal(&raw_ff, &utf8_y_diaeresis));
    }

    #[test]
    fn concat_preserves_high_bytes_and_uses_the_string_pool() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let mut state = LuaState::new();
        let left = byte_value(&mut gc, &mut pool, &[0x00, 0x80, 0xff]);
        let right = byte_value(&mut gc, &mut pool, &[0xfe, 0xc3, 0x00]);
        let expected = [0x00, 0x80, 0xff, 0xfe, 0xc3, 0x00];

        let result_ref =
            crate::state::with_vm_context_gc(&mut state, &mut gc, &mut pool, |state, gc| {
                ensure_stack_slot(state, 0);
                let outcome = exec_concat_into(
                    state,
                    gc,
                    &left,
                    &right,
                    0,
                    VmMetamethodContinuation::CompleteSet {
                        proto: GcRef::null(),
                        pc: 0,
                    },
                )
                .expect("byte concat succeeds");
                assert!(matches!(outcome, VmMetamethodOutcome::Ready(())));
                let result = state.stack.at(0).cloned().unwrap_or(Value::Nil);
                let Value::String(result_ref) = result else {
                    panic!("concat must return a Lua string");
                };
                gc.with_string_bytes(result_ref, |bytes| assert_eq!(bytes, expected))
                    .expect("non-null string");
                result_ref
            });
        assert_eq!(pool.find_bytes(&expected), Some(result_ref));
    }

    #[test]
    fn embedded_nul_participates_in_length_and_equality() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let first = byte_value(&mut gc, &mut pool, b"a\0b");
        let same = byte_value(&mut gc, &mut pool, b"a\0b");
        let different_tail = byte_value(&mut gc, &mut pool, b"a\0c");

        assert_eq!(
            gc.with_ref(
                match &first {
                    Value::String(string) => *string,
                    _ => unreachable!("fixture is a string"),
                },
                |string| Value::Number(string.len() as f64),
            )
            .expect("string remains live"),
            Value::Number(3.0),
        );
        assert!(values_equal(&first, &same));
        assert!(!values_equal(&first, &different_tail));
    }

    #[test]
    fn oracle_ordering_compares_each_embedded_nul_segment() {
        assert_eq!(compare_lua_string_bytes(b"a\0b", b"a\0c"), Ordering::Less);
        assert_eq!(compare_lua_string_bytes(b"a", b"a\0"), Ordering::Less);
        assert_eq!(compare_lua_string_bytes(b"a\0", b"a"), Ordering::Greater);
        assert_eq!(
            compare_lua_string_bytes(&[0xff], &[0xc3, 0xbf]),
            Ordering::Greater
        );

        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let mut state = LuaState::new();
        let left = byte_value(&mut gc, &mut pool, b"a\0b");
        let right = byte_value(&mut gc, &mut pool, b"a\0c");
        crate::state::with_vm_context_gc(&mut state, &mut gc, &mut pool, |state, gc| {
            let continuation = VmMetamethodContinuation::CompleteSet {
                proto: GcRef::null(),
                pc: 0,
            };
            assert!(matches!(
                exec_lt(state, gc, &left, &right, continuation.clone()).expect("strings compare"),
                VmMetamethodOutcome::Ready(true)
            ));
            assert!(matches!(
                exec_le(state, gc, &left, &right, continuation).expect("strings compare"),
                VmMetamethodOutcome::Ready(true)
            ));
        });
    }

    #[test]
    fn invalid_utf8_numeric_string_fails_without_panicking() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let invalid = byte_value(&mut gc, &mut pool, &[b' ', 0xff, b'1', b' ']);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            to_arith_number(&gc, &invalid)
        }));
        assert_eq!(result.expect("numeric conversion must not panic"), None);
        assert_eq!(as_number(&gc, &invalid), 0.0);
    }

    #[test]
    fn protected_call_results_are_published_before_stack_window_restores() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let function = gc.create(Function::new_c(return_fresh_string));
        let mut state = LuaState::new();

        crate::state::with_vm_context_gc(&mut state, &mut gc, &mut string_pool, |state, gc| {
            call_value_with_results(
                state,
                gc,
                Value::Function(function),
                &[],
                Some(1),
                |state, gc, results| {
                    assert_eq!(state.top, 0);
                    assert_eq!(gc.temporary_root_count(), 1);
                    let report = gc.begin_mark_only();
                    assert_eq!(report.temporary_seeded, 1);
                    state.push_value(results[0].clone());
                },
            )
            .expect("fresh result publishes to the restored caller stack");

            assert_eq!(gc.temporary_root_count(), 0);
            assert!(matches!(state.stack.at(0), Some(Value::String(_))));
        });
    }
}

#[cfg(test)]
mod proto_handle_tests {

    use lua_compiler::codegen::CodeGenerator;
    use lua_compiler::parser::Parser;
    use lua_core::string_pool::StringPool;

    use super::*;

    unsafe extern "C" fn test_noop_c_function(_: *mut std::ffi::c_void) -> i32 {
        0
    }

    fn compile(
        gc: &mut GarbageCollector,
        string_pool: &mut StringPool,
        source: &str,
    ) -> GcRef<Proto> {
        let mut parser = Parser::new(source);
        let chunk = parser.parse().expect("test source parses");
        let proto = CodeGenerator::new_with_pool(gc, string_pool)
            .generate(&chunk, "<proto-handle-test>")
            .expect("test source compiles");
        gc.create(proto)
    }

    #[test]
    fn execute_rejects_stale_and_foreign_proto_handles() {
        let mut stale_gc = GarbageCollector::new();
        let stale = stale_gc.create(Proto::new());
        stale_gc.destroy_all(&mut StringPool::new());

        let mut state = LuaState::new();
        let stale_error = execute_proto(&mut state, stale, &mut stale_gc)
            .expect_err("destroyed Proto must be rejected");
        assert!(stale_error.message.contains("not live"));

        let mut foreign_gc = GarbageCollector::new();
        let foreign = foreign_gc.create(Proto::new());
        let mut local_gc = GarbageCollector::new();
        let foreign_error = execute_proto(&mut state, foreign, &mut local_gc)
            .expect_err("cross-collector Proto must be rejected");
        assert!(foreign_error.message.contains("not live"));
        assert!(state.call_stack.iter().all(|ci| ci.proto.is_none()));
    }

    #[test]
    fn return_and_error_clear_inactive_proto_and_vararg_frames() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let mut returned = LuaState::new();
        let return_proto = compile(
            &mut gc,
            &mut string_pool,
            "local function f(...) return 42 end return f(1, 2)",
        );
        crate::state::with_vm_context_gc(
            &mut returned,
            &mut gc,
            &mut string_pool,
            |returned, gc| {
                assert!(matches!(
                    execute_proto(returned, return_proto, gc),
                    Ok(VmExit::Complete(ExecResult::Returned))
                ));
            },
        );
        assert_eq!(returned.current_ci, 0);
        assert!(
            returned
                .call_stack
                .iter()
                .all(|ci| ci.proto.is_none() && ci.varargs.is_empty()),
            "{:?}",
            returned.call_stack
        );

        let mut errored = LuaState::new();
        let error_proto = compile(
            &mut gc,
            &mut string_pool,
            "local function f(...) return nil + 1 end return f(1, 2)",
        );
        crate::state::with_vm_context_gc(&mut errored, &mut gc, &mut string_pool, |errored, gc| {
            execute_proto(errored, error_proto, gc)
                .expect_err("nested arithmetic error must propagate");
        });
        assert_eq!(errored.current_ci, 0);
        assert!(
            errored
                .call_stack
                .iter()
                .all(|ci| ci.proto.is_none() && ci.varargs.is_empty())
        );
    }

    #[test]
    fn debug_line_skip_is_one_shot_and_matches_full_object_identity() {
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let mut first = Proto::new();
        first.add_line_info(17);
        let first = gc.create(first);
        let mut second = Proto::new();
        second.add_line_info(17);
        let second = gc.create(second);
        assert_ne!(first.object_id(), second.object_id());

        let hook = gc.create(Function::new_c(test_noop_c_function));
        let mut state = LuaState::new();
        state.debug_hook = Some(Value::Function(hook));
        state.debug_hook_mask = "l".to_string();
        state.debug_hook_skip_proto = Some(first);
        state.debug_hook_skip_line = 17;

        crate::state::with_vm_context_gc(&mut state, &mut gc, &mut string_pool, |state, gc| {
            let different = state
                .with_native_request_scope(|state| {
                    let request_id = run_debug_instruction_hooks(state, gc, second, 0)?;
                    let request_id = request_id.expect("different Proto publishes the hook");
                    assert!(state.take_native_request(request_id).is_some());
                    state.debug_hook_active = false;
                    Ok::<_, RuntimeError>(())
                })
                .expect("native request scope opens");
            different.expect("different Proto identity runs the hook");
            assert_eq!(state.debug_hook_skip_proto, Some(first));

            let matching = state
                .with_native_request_scope(|state| {
                    assert!(run_debug_instruction_hooks(state, gc, first, 0)?.is_none());
                    Ok::<_, RuntimeError>(())
                })
                .expect("native request scope opens");
            matching.expect("matching Proto consumes the skip");
            assert_eq!(state.debug_hook_skip_proto, None);
            assert_eq!(state.debug_hook_skip_line, -1);

            let consumed = state
                .with_native_request_scope(|state| {
                    let request_id = run_debug_instruction_hooks(state, gc, first, 0)?;
                    let request_id = request_id.expect("consumed skip publishes the hook");
                    assert!(state.take_native_request(request_id).is_some());
                    state.debug_hook_active = false;
                    Ok::<_, RuntimeError>(())
                })
                .expect("native request scope opens");
            consumed.expect("consumed skip no longer suppresses the line");
        });
    }
}
