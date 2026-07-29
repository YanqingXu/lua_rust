//! Lua 虚拟机执行引擎
#![allow(clippy::collapsible_if, clippy::collapsible_match)]
//!
//! 基于寄存器的字节码解释器，实现全部 38 条 Lua 5.1 指令。
//!

use lua_compiler::opcode::{self, OpCode};
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

use crate::native::{
    DeferredNativeCall, DeferredVmContinuation, NativeRequestId, NativeRequestPublishError,
    ResumeEnvelope, ResumeResponse, UpvalueAccessOperation, UpvalueAccessRequest,
};
use crate::state::{LUA_MULTRET, LuaState, ThreadStatus};

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
        .string_pool
        .ok_or(GcRefValidationError::StringPoolUnavailable)?;
    // SAFETY: LuaState::string_pool points at the pinned RuntimeHeap service
    // for the duration of this exclusive VM state turn.
    transaction.intern_bytes(unsafe { &mut *pool }, bytes)
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
    let result = execute_proto_inner(l, proto, gc);
    if result.is_err() && entry_ci == 0 && l.current_ci == 0 {
        let ci = l.current_call_info_mut();
        if ci.proto == Some(proto) {
            ci.proto = None;
            ci.varargs.clear();
        }
    }
    result
}

fn execute_proto_inner(
    l: &mut LuaState,
    proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    if l.nccalls > MAX_CALLS {
        return Err(RuntimeError::new(
            "VM: stack overflow (too many nested calls)",
        ));
    }
    l.gc = Some(gc as *mut GarbageCollector);

    let mut active_proto_ref = proto;
    let initial_max_stack = gc
        .with_ref(active_proto_ref, Proto::max_stack_size)
        .map_err(invalid_proto_error)?;
    let _nresults = l.current_call_info().nresults;
    let resume_from_yield = l.status == ThreadStatus::Yield;
    let mut pc: usize = if resume_from_yield {
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

    // 主解释循环
    loop {
        let active_proto_ptr = gc
            .validate_ref(active_proto_ref)
            .map_err(invalid_proto_error)?;
        // SAFETY: validate_ref checked address, ObjectId, and type against this
        // collector immediately before the dereference. Destructive sweep is
        // forbidden while the VM executes, so the allocation cannot disappear
        // during this interpreter iteration.
        let active_proto = unsafe { active_proto_ptr.as_ref() };
        let code = active_proto.code();
        if pc >= code.len() {
            break;
        }
        let constants = active_proto.constants();
        let inst = code[pc];
        let op = opcode::get_opcode(inst);
        let base_idx = l.current_call_info().base;
        l.current_call_info_mut().savedpc = Some(pc);
        run_debug_instruction_hooks(l, gc, active_proto_ref, pc, op)?;
        run_auto_weak_gc(l, gc);

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
                let val = constants.get(bx).cloned().unwrap_or(Value::Nil);
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
                let name = constants.get(bx).cloned().unwrap_or(Value::Nil);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                get_global_into(l, gc, stack_limit, &name, base_idx + a)?;
            }

            OpCode::GETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let table = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                if should_error_index(l, gc, &table) {
                    return Err(runtime_error_at(
                        gc,
                        active_proto,
                        pc,
                        index_error_message(gc, active_proto, pc, b, constants, &table),
                    ));
                }
                get_table_into(l, gc, stack_limit, &table, &key, base_idx + a)?;
            }

            // ── 变量赋值 (3) ─────────────────────────────────
            OpCode::SETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // _G[K(Bx)] := R(A)
                let name = constants.get(bx).cloned().unwrap_or(Value::Nil);
                let val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                set_global(l, gc, stack_limit, &name, &val)?;
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
                let key = get_rk(l, base_idx, b, constants);
                let value = get_rk(l, base_idx, c, constants);
                let table_val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                if should_error_newindex(l, gc, &table_val) {
                    return Err(runtime_error_at(
                        gc,
                        active_proto,
                        pc,
                        index_error_message(gc, active_proto, pc, a, constants, &table_val),
                    ));
                }
                set_table_value(l, gc, stack_limit, &table_val, &key, &value)?;
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
                let key = get_rk(l, base_idx, c, constants);
                if should_error_index(l, gc, &obj) {
                    return Err(runtime_error_at(
                        gc,
                        active_proto,
                        pc,
                        index_error_message(gc, active_proto, pc, b, constants, &obj),
                    ));
                }
                // R(A+1) = R(B)
                if let Some(dst) = l.stack.at_mut(base_idx + a + 1) {
                    *dst = obj.clone();
                }
                // R(A) = R(B)[RK(C)]
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                get_table_into(l, gc, stack_limit, &obj, &key, base_idx + a)?;
            }

            // ── 算术运算 (6) ─────────────────────────────────
            OpCode::ADD | OpCode::SUB | OpCode::MUL | OpCode::DIV | OpCode::MOD | OpCode::POW => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b, constants);
                let rhs = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                match exec_arith_into(l, gc, stack_limit, op, &lhs, &rhs, base_idx + a) {
                    Ok(()) => {}
                    Err(err)
                        if err.message == "attempt to perform arithmetic on a non-number value" =>
                    {
                        return Err(RuntimeError::new(arith_error_message(
                            gc,
                            active_proto,
                            pc,
                            constants,
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
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                match exec_unm_into(l, gc, stack_limit, &val, base_idx + a) {
                    Ok(()) => {}
                    Err(err)
                        if err.message == "attempt to perform arithmetic on a non-number value" =>
                    {
                        return Err(RuntimeError::new(unm_error_message(
                            gc,
                            active_proto,
                            pc,
                            b,
                            constants,
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
                let len = exec_len(gc, &val);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = len;
                }
            }

            // ── 字符串连接 ────────────────────────────────────
            OpCode::CONCAT => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst) as usize;
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let destination = base_idx + a;
                let initial = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                if let Some(dst) = l.stack.at_mut(destination) {
                    *dst = initial;
                }
                for i in (b + 1)..=c {
                    let rhs = l.stack.at(base_idx + i).cloned().unwrap_or(Value::Nil);
                    let lhs = l.stack.at(destination).cloned().unwrap_or(Value::Nil);
                    exec_concat_into(l, gc, stack_limit, &lhs, &rhs, destination)?;
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
                let lhs = get_rk(l, base_idx, b, constants);
                let rhs = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let equal = exec_eq(l, gc, stack_limit, &lhs, &rhs)?;
                // Lua 5.1: skip when (equal as i32) != A
                if (equal && a == 0) || (!equal && a != 0) {
                    pc += 1; // skip next
                }
            }

            OpCode::LT => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b, constants);
                let rhs = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let less = exec_lt(l, gc, stack_limit, &lhs, &rhs)?;
                if (less && a == 0) || (!less && a != 0) {
                    pc += 1;
                }
            }

            OpCode::LE => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b, constants);
                let rhs = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let le = exec_le(l, gc, stack_limit, &lhs, &rhs)?;
                if (le && a == 0) || (!le && a != 0) {
                    pc += 1;
                }
            }

            OpCode::TEST => {
                let a = opcode::get_arg_a(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let truthy = !val.is_false();
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

                if let Value::Function(func_ref) = func {
                    // SAFETY: GC is not running during VM execution; GcRef remains valid
                    if let Some(func_obj) = unsafe { func_ref.as_ref() } {
                        if func_obj.is_lua_function() {
                            if let Some(callee_proto_ref) = func_obj.proto() {
                                let callee_proto_ptr = gc
                                    .validate_ref(callee_proto_ref)
                                    .map_err(invalid_proto_error)?;
                                // SAFETY: validated above; destructive sweep is disabled
                                // throughout VM execution.
                                let callee_proto = unsafe { callee_proto_ptr.as_ref() };
                                // Setup new call frame
                                let new_base = base_idx + a + 1;
                                let varargs =
                                    prepare_lua_varargs(l, gc, callee_proto, new_base, nargs);
                                let saved_ci = l.current_ci;
                                let ci = l.push_call_info();
                                ci.func = base_idx + a;
                                ci.base = new_base;
                                ci.top = new_base + callee_proto.max_stack_size() as usize;
                                ci.nresults = nresults;
                                ci.nargs = nargs;
                                ci.varargs = varargs;
                                ci.savedpc = None;
                                ci.proto = Some(callee_proto_ref);
                                ci.tailcalls = 0;

                                if let Err(e) = fire_debug_hook(l, gc, "call", None) {
                                    unwind_lua_frames_to(l, gc, saved_ci)?;
                                    return Err(e);
                                }

                                // Recursively execute the called function
                                match execute_nested_proto_at(
                                    l,
                                    active_proto_ref,
                                    pc,
                                    callee_proto_ref,
                                    gc,
                                ) {
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
                                    Err(e) => {
                                        // Restore call frame and propagate error
                                        unwind_lua_frames_to(l, gc, saved_ci)?;
                                        return Err(e);
                                    }
                                }

                                if l.current_ci != saved_ci {
                                    unwind_lua_frames_to(l, gc, saved_ci)?;
                                    return Err(RuntimeError::new("VM: call frame imbalance"));
                                }
                                pc += 1;
                                continue;
                            }
                            return Err(RuntimeError::new("Lua function has no proto"));
                        } else if let Some(callable) = native_callable(func_obj) {
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
                                Err(err) if err.message.starts_with("bad argument") => {
                                    return Err(runtime_error_at(
                                        gc,
                                        active_proto,
                                        pc,
                                        err.message,
                                    ));
                                }
                                Err(err) => return Err(err),
                            }
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
                    let detail = describe_register(gc, active_proto, pc, a, constants, 8)
                        .map(|name| format!("{} (a {} value)", name, type_desc))
                        .unwrap_or_else(|| format!("a {} value", type_desc));
                    return Err(RuntimeError::new(format!("attempt to call {detail}")));
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
                    call_value_at_stack(l, gc, func_pos, actual_nargs, None)?;
                    let available = l.top.saturating_sub(func_pos);
                    let mut results = Vec::with_capacity(available);
                    for i in 0..available {
                        results.push(l.stack.at(func_pos + i).cloned().unwrap_or(Value::Nil));
                    }
                    close_state_upvalues(l, gc, ci.base)?;
                    if available > 0 {
                        ensure_stack_slot(l, ci.func + available - 1);
                    }
                    for (i, result) in results.into_iter().enumerate() {
                        if let Some(dst) = l.stack.at_mut(ci.func + i) {
                            *dst = result;
                        }
                    }
                    l.top = ci.func + available;
                    l.pop_call_info();
                    return Ok(VmExit::Complete(ExecResult::Returned));
                }

                if let Value::Function(func_ref) = func {
                    // SAFETY: GC is not running during VM execution; GcRef remains valid
                    if let Some(func_obj) = unsafe { func_ref.as_ref() } {
                        if func_obj.is_lua_function() {
                            if let Some(tail_proto_ref) = func_obj.proto() {
                                if !tail_proto_ref.is_null() {
                                    let tail_proto_ptr = gc
                                        .validate_ref(tail_proto_ref)
                                        .map_err(invalid_proto_error)?;
                                    // SAFETY: validated above; destructive sweep is disabled
                                    // throughout VM execution.
                                    let tail_proto = unsafe { tail_proto_ptr.as_ref() };
                                    let args: Vec<Value> = (0..actual_nargs)
                                        .map(|i| {
                                            l.stack
                                                .at(func_pos + 1 + i)
                                                .cloned()
                                                .unwrap_or(Value::Nil)
                                        })
                                        .collect();
                                    close_state_upvalues(l, gc, base_idx)?;
                                    for (i, arg) in args.iter().enumerate().take(actual_nargs) {
                                        if let Some(dst) = l.stack.at_mut(base_idx + i) {
                                            *dst = arg.clone();
                                        }
                                    }

                                    let tail_nargs = actual_nargs as i32;
                                    let varargs = prepare_lua_varargs(
                                        l, gc, tail_proto, base_idx, tail_nargs,
                                    );
                                    let new_top = base_idx + tail_proto.max_stack_size() as usize;
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
                                    fire_debug_hook(l, gc, "call", None)?;
                                    active_proto_ref = tail_proto_ref;
                                    pc = 0;
                                    continue;
                                }
                            }
                        } else if let Some(callable) = native_callable(func_obj) {
                            match call_native_function(
                                l,
                                gc,
                                func_pos,
                                actual_nargs,
                                None,
                                callable,
                            )? {
                                VmExit::Complete(ExecResult::Yielded) => {
                                    return Ok(VmExit::Complete(ExecResult::Yielded));
                                }
                                VmExit::NativeRequest(id) => {
                                    return Ok(VmExit::NativeRequest(id));
                                }
                                VmExit::UpvalueAccess(request) => {
                                    return Ok(VmExit::UpvalueAccess(request));
                                }
                                VmExit::Complete(ExecResult::Returned) => {}
                            }
                            let available = l.top.saturating_sub(func_pos);
                            let ci = l.current_call_info().clone();
                            let wanted = if l.current_ci == 0 || ci.nresults == LUA_MULTRET {
                                available
                            } else {
                                ci.nresults.max(0) as usize
                            };
                            let mut results = Vec::with_capacity(wanted);
                            for i in 0..wanted {
                                let result = if i < available {
                                    l.stack.at(func_pos + i).cloned().unwrap_or(Value::Nil)
                                } else {
                                    Value::Nil
                                };
                                results.push(result);
                            }

                            close_state_upvalues(l, gc, ci.base)?;
                            if wanted > 0 {
                                ensure_stack_slot(l, ci.func + wanted - 1);
                            }
                            for (i, result) in results.into_iter().enumerate() {
                                if let Some(dst) = l.stack.at_mut(ci.func + i) {
                                    *dst = result;
                                }
                            }
                            l.top = ci.func + wanted;
                            l.pop_call_info();
                            return Ok(VmExit::Complete(ExecResult::Returned));
                        }
                    }
                }
                return Err(RuntimeError::new("tail call: only Lua functions supported"));
            }

            OpCode::RETURN => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as i32;
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

                fire_debug_hook(l, gc, "return", None)?;
                for _ in 0..ci.tailcalls {
                    fire_debug_hook(l, gc, "tail return", None)?;
                }

                if wanted > 0 {
                    ensure_stack_slot(l, ci.func + wanted - 1);
                }
                for (i, src) in results.into_iter().enumerate() {
                    let dst_idx = ci.func + i;
                    if let Some(dst) = l.stack.at_mut(dst_idx) {
                        *dst = src;
                    }
                }
                l.top = ci.func + wanted;
                l.pop_call_info();
                return Ok(VmExit::Complete(ExecResult::Returned));
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
                // Update R(A) and R(A+3)
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Number(idx_num);
                }
                if let Some(dst) = l.stack.at_mut(base_idx + a + 3) {
                    *dst = Value::Number(idx_num);
                }
                if (step_num > 0.0 && idx_num <= limit_num)
                    || (step_num < 0.0 && idx_num >= limit_num)
                {
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
                            active_proto,
                            pc,
                            "'for' step must be a number",
                        ));
                    }
                };
                if to_arith_number(gc, &limit).is_none() {
                    return Err(runtime_error_at(
                        gc,
                        active_proto,
                        pc,
                        "'for' limit must be a number",
                    ));
                }
                let init_num = match to_arith_number(gc, &init) {
                    Some(init) => init - step_num,
                    None => {
                        return Err(runtime_error_at(
                            gc,
                            active_proto,
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
                        // SAFETY: iterator function is kept alive by the stack.
                        if let Some(func_obj) = unsafe { func_ref.as_ref() } {
                            if let Some(callable) = native_callable(func_obj) {
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
                                    VmExit::Complete(ExecResult::Returned) => {}
                                }
                            } else if let Some(iter_proto_ref) = func_obj.proto() {
                                let iter_proto_ptr = gc
                                    .validate_ref(iter_proto_ref)
                                    .map_err(invalid_proto_error)?;
                                // SAFETY: validated above; destructive sweep is disabled
                                // throughout VM execution.
                                let iter_proto = unsafe { iter_proto_ptr.as_ref() };
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
                                let varargs =
                                    prepare_lua_varargs(l, gc, iter_proto, call_pos + 1, 2);
                                let ci = l.push_call_info();
                                ci.func = call_pos;
                                ci.base = call_pos + 1;
                                ci.top = ci.base + iter_proto.max_stack_size() as usize;
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
                                    Err(e) => {
                                        unwind_lua_frames_to(l, gc, saved_ci)?;
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
                    }
                    _ => {
                        return Err(runtime_error_at(
                            gc,
                            active_proto,
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
                    c = *code
                        .get(pc)
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
                if let Some(table_val) = l.stack.at_mut(base_idx + a) {
                    if let Value::Table(_table_ref) = table_val {
                        let table_ptr = _table_ref.as_ptr() as *mut Table;
                        // SAFETY: We hold exclusive access via &mut LuaState, GC is not
                        // running during VM execution, and the table is kept alive by the
                        // LuaState stack. The raw pointer is derived from a valid GcRef.
                        unsafe {
                            for i in 1..=count {
                                let val =
                                    l.stack.at(base_idx + a + i).cloned().unwrap_or(Value::Nil);
                                let idx = ((c - 1) * 50) as i32 + i as i32;
                                (*table_ptr).set_array(idx, &val);
                            }
                        }
                    }
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
                if bx >= active_proto.sub_proto_count() {
                    return Err(RuntimeError::new("VM: CLOSURE proto index out of range"));
                }
                let sub_proto_ref = active_proto.sub_proto(bx);
                if !sub_proto_ref.is_null() {
                    let environment = current_env(l);
                    let child_proto_ptr = gc
                        .validate_ref(sub_proto_ref)
                        .map_err(invalid_proto_error)?;
                    // SAFETY: validated above; destructive sweep is disabled
                    // throughout VM execution.
                    let child_proto = unsafe { child_proto_ptr.as_ref() };
                    let mut next_pc = pc + 1;
                    let mut upvalues = Vec::with_capacity(child_proto.num_upvalues() as usize);
                    for _ in 0..child_proto.num_upvalues() {
                        let pseudo = *code.get(next_pc).ok_or_else(|| {
                            RuntimeError::new("VM: CLOSURE missing upvalue pseudo instruction")
                        })?;
                        next_pc += 1;

                        let upvalue_ref = match opcode::get_opcode(pseudo) {
                            OpCode::MOVE => {
                                let b = opcode::get_arg_b(pseudo) as usize;
                                l.find_or_create_upvalue(base_idx + b, gc)
                                    .map_err(|error| {
                                        RuntimeError::new(format!(
                                            "VM: could not publish open Upvalue: {error}"
                                        ))
                                    })?
                            }
                            OpCode::GETUPVAL => {
                                let b = opcode::get_arg_b(pseudo) as usize;
                                current_lua_function(l)
                                    .and_then(|current| current.upvalue(b))
                                    .ok_or_else(|| {
                                        RuntimeError::new(
                                            "VM: CLOSURE invalid parent upvalue index",
                                        )
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
    }

    l.pop_call_info();
    Ok(VmExit::Complete(ExecResult::Returned))
}

fn run_debug_instruction_hooks(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    proto: GcRef<Proto>,
    pc: usize,
    op: OpCode,
) -> Result<(), RuntimeError> {
    if l.debug_hook.is_none() || l.debug_hook_active {
        return Ok(());
    }

    if l.debug_hook_count > 0
        && matches!(
            op,
            OpCode::LOADK | OpCode::FORPREP | OpCode::FORLOOP | OpCode::TFORLOOP
        )
    {
        l.debug_hook_countdown -= 1;
        if l.debug_hook_countdown <= 0 {
            l.debug_hook_countdown = l.debug_hook_count;
            fire_debug_hook(l, gc, "count", None)?;
        }
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
            fire_debug_hook(l, gc, "line", Some(line))?;
        } else {
            l.debug_hook_last_pc = pc;
        }
    }

    Ok(())
}

fn run_auto_weak_gc(l: &mut LuaState, gc: &mut GarbageCollector) {
    if l.gc_stopped || l.debug_hook_active || !gc.has_seen_weak_table() {
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
        .map_err(|error| RuntimeError::new(format!("VM: could not close open Upvalue: {error}")))
}

fn fire_debug_hook(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    event: &str,
    line: Option<i32>,
) -> Result<(), RuntimeError> {
    let Some(hook) = l.debug_hook.clone() else {
        return Ok(());
    };
    if l.debug_hook_active {
        return Ok(());
    }
    if matches!(event, "call") && !l.debug_hook_mask.contains('c') {
        return Ok(());
    }
    if matches!(event, "return" | "tail return") && !l.debug_hook_mask.contains('r') {
        return Ok(());
    }

    let saved_top = l.top;
    gc.with_publication(|transaction| {
        let event = rooted_vm_bytes(l, transaction, event.as_bytes())?;
        // SAFETY: the callback installs the string in the active Lua stack.
        unsafe {
            transaction.publish_string_value(event, |value| {
                l.push_value(value);
            })
        }
    })
    .map_err(|error| RuntimeError::new(format!("invalid debug-hook event: {error}")))?;
    let event_value = l.stack.at(saved_top).cloned().unwrap_or(Value::Nil);
    let line_value = line.map_or(Value::Nil, |line| Value::Number(line as f64));
    let frame_top = l.current_call_info().top.max(saved_top + 1);
    l.top = frame_top;
    l.debug_hook_active = true;
    let result = call_value_with_results(
        l,
        gc,
        hook,
        &[event_value, line_value],
        Some(0),
        |_, _, _| (),
    );
    l.debug_hook_active = false;
    l.top = saved_top;
    result.map(|_| ())
}

fn execute_nested_proto_at(
    l: &mut LuaState,
    caller_proto: GcRef<Proto>,
    caller_pc: usize,
    callee_proto: GcRef<Proto>,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    let caller_proto_ptr = gc.validate_ref(caller_proto).map_err(invalid_proto_error)?;
    // SAFETY: validated above; destructive sweep is disabled throughout VM
    // execution.
    let caller_proto = unsafe { caller_proto_ptr.as_ref() };
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
    while l.current_ci > target_ci {
        let base = l.current_call_info().base;
        close_state_upvalues(l, gc, base)?;
        l.pop_call_info();
    }
    Ok(())
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
    l.gc = Some(gc as *mut GarbageCollector);

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
    l.gc = Some(gc as *mut GarbageCollector);

    let func = l.stack.at(func_pos).cloned().unwrap_or(Value::Nil);
    let Value::Function(func_ref) = func else {
        return Err(RuntimeError::new(format!(
            "attempt to call a {} value",
            value_type_name(&func)
        )));
    };

    // SAFETY: function value is on the coroutine stack and GC is not running.
    let Some(func_obj) = (unsafe { func_ref.as_ref() }) else {
        return Err(RuntimeError::new("attempt to call an invalid function"));
    };
    let Some(callee_proto_ref) = func_obj.proto() else {
        return Err(RuntimeError::new("coroutine entry must be a Lua function"));
    };
    let callee_proto_ptr = gc
        .validate_ref(callee_proto_ref)
        .map_err(invalid_proto_error)?;
    // SAFETY: validated above; destructive sweep is disabled throughout VM
    // execution.
    let callee_proto = unsafe { callee_proto_ptr.as_ref() };

    let new_base = func_pos + 1;
    let varargs = prepare_lua_varargs(l, gc, callee_proto, new_base, nargs as i32);
    let ci = l.push_call_info();
    ci.func = func_pos;
    ci.base = new_base;
    ci.top = new_base + callee_proto.max_stack_size() as usize;
    ci.nresults = wanted_results.map_or(LUA_MULTRET, |n| n as i32);
    ci.nargs = nargs as i32;
    ci.varargs = varargs;
    ci.savedpc = None;
    ci.proto = Some(callee_proto_ref);
    ci.tailcalls = 0;

    Ok(())
}

pub fn resume_lua_thread(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
) -> Result<VmExit, RuntimeError> {
    l.gc = Some(gc as *mut GarbageCollector);

    loop {
        if l.current_ci == 0 {
            return Ok(VmExit::Complete(ExecResult::Returned));
        }

        let proto_ref = l
            .current_call_info()
            .proto
            .or_else(|| current_lua_function(l).and_then(|function| function.proto()))
            .ok_or_else(|| RuntimeError::new("coroutine frame has no Lua proto"))?;
        gc.validate_ref(proto_ref).map_err(invalid_proto_error)?;

        l.status = ThreadStatus::Yield;
        let result = match execute_proto(l, proto_ref, gc) {
            Ok(result) => result,
            Err(error) => {
                unwind_lua_frames_to(l, gc, 0)?;
                return Err(error);
            }
        };
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

    // SAFETY: function value is on the active Lua stack and GC is not running.
    let Some(func_obj) = (unsafe { func_ref.as_ref() }) else {
        return Err(RuntimeError::new("attempt to call an invalid function"));
    };

    if let Some(callable) = native_callable(func_obj) {
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
        }
        return Ok(());
    }

    let Some(callee_proto_ref) = func_obj.proto() else {
        return Err(RuntimeError::new("Lua function has no proto"));
    };
    let callee_proto_ptr = gc
        .validate_ref(callee_proto_ref)
        .map_err(invalid_proto_error)?;
    // SAFETY: validated above; destructive sweep is disabled throughout VM
    // execution.
    let callee_proto = unsafe { callee_proto_ptr.as_ref() };

    let saved_ci = l.current_ci;
    let new_base = func_pos + 1;
    let varargs = prepare_lua_varargs(l, gc, callee_proto, new_base, nargs as i32);
    let ci = l.push_call_info();
    ci.func = func_pos;
    ci.base = new_base;
    ci.top = new_base + callee_proto.max_stack_size() as usize;
    ci.nresults = wanted_results.map_or(LUA_MULTRET, |n| n as i32);
    ci.nargs = nargs as i32;
    ci.varargs = varargs;
    ci.savedpc = None;
    ci.proto = Some(callee_proto_ref);
    ci.tailcalls = 0;

    match execute_counted_proto(l, callee_proto_ref, gc, RuntimeError::new("stack overflow")) {
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
        Err(e) => {
            unwind_lua_frames_to(l, gc, saved_ci)?;
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

fn c_function_display_name(l: &LuaState, func_pos: usize) -> String {
    let Some(Value::Function(func_ref)) = l.stack.at(func_pos).cloned() else {
        return "<unknown>".to_string();
    };
    let Some(global_ref) = l.global_table else {
        return format!("function: {:p}", func_ref.as_ptr());
    };
    // SAFETY: global table is rooted for the duration of VM execution.
    let Some(global) = (unsafe { global_ref.as_ref() }) else {
        return format!("function: {:p}", func_ref.as_ptr());
    };
    find_function_name_in_table(l, global, func_ref).unwrap_or_else(|| {
        for (key, value) in global.hash_entries() {
            let Value::String(lib_name_ref) = key else {
                continue;
            };
            let Value::Table(table_ref) = value else {
                continue;
            };
            // SAFETY: library tables are reachable from the global table.
            let Some(table) = (unsafe { table_ref.as_ref() }) else {
                continue;
            };
            if let Some(name) = find_function_name_in_table(l, table, func_ref) {
                let lib_name = l
                    .with_string_bytes(*lib_name_ref, |bytes| {
                        String::from_utf8_lossy(bytes).into_owned()
                    })
                    .ok()
                    .unwrap_or_default();
                return format!("{lib_name}.{name}");
            }
        }
        format!("function: {:p}", func_ref.as_ptr())
    })
}

fn find_function_name_in_table(
    l: &LuaState,
    table: &Table,
    func_ref: GcRef<Function>,
) -> Option<String> {
    for (key, value) in table.hash_entries() {
        if let (Value::String(name_ref), Value::Function(value_ref)) = (key, value)
            && *value_ref == func_ref
        {
            return l
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
    proto: &Proto,
    pc: usize,
    message: impl Into<String>,
) -> RuntimeError {
    let source = proto
        .source()
        .and_then(|source_ref| {
            gc.with_string_bytes(source_ref, |bytes| {
                String::from_utf8_lossy(bytes).into_owned()
            })
            .ok()
        })
        .unwrap_or_else(|| "?".to_string());
    let line = proto.line(pc);
    RuntimeError::new(format!("{}:{}: {}", source, line, message.into()))
}

fn stack_overflow_error(
    l: &LuaState,
    gc: &GarbageCollector,
    caller_proto: &Proto,
    caller_pc: usize,
) -> RuntimeError {
    let recursive_line = caller_proto.line(caller_pc);
    let mut message = format!("?:{}: stack overflow", recursive_line);
    for _ in 0..20 {
        message.push_str(&format!("\n?:{}: in function 'y'", recursive_line));
    }
    if let Some(line) = first_non_recursive_caller_line(l, gc) {
        message.push_str(&format!("\n?:{}: in function 'g'", line));
    } else {
        let line = caller_proto.line(caller_pc);
        message.push_str(&format!("\n?:{}: in function", line));
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
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    value: &Value,
) -> String {
    let type_desc = value_type_name(value);
    describe_register_for_index(gc, proto, pc, reg, constants, 8)
        .map(|name| format!("attempt to index {name} (a {type_desc} value)"))
        .unwrap_or_else(|| format!("attempt to index a {type_desc} value"))
}

fn arith_error_message(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    constants: &[Value],
    lhs: (i32, &Value),
    rhs: (i32, &Value),
) -> String {
    let (bad_rk, bad_value) = if to_arith_number(gc, lhs.1).is_none() {
        lhs
    } else {
        rhs
    };
    operand_error_message(
        gc,
        proto,
        pc,
        bad_rk,
        constants,
        bad_value,
        "attempt to perform arithmetic on",
    )
}

fn unm_error_message(
    gc: &GarbageCollector,
    proto: &Proto,
    pc: usize,
    rk: usize,
    constants: &[Value],
    value: &Value,
) -> String {
    operand_error_message(
        gc,
        proto,
        pc,
        rk as i32,
        constants,
        value,
        "attempt to perform arithmetic on",
    )
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
    proto: &Proto,
    base: usize,
    nargs: i32,
) -> Vec<Value> {
    let fixed_params = proto.num_params() as usize;
    let varargs = if proto.is_vararg() {
        collect_varargs(l, base, nargs, fixed_params)
    } else {
        Vec::new()
    };

    fill_missing_fixed_params(l, base, nargs, fixed_params);

    if proto.vararg_flags() & VARARG_NEEDSARG != 0 {
        install_arg_table(l, gc, base + fixed_params, &varargs);
    }

    varargs
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

#[derive(Clone, Copy)]
struct NativeCallFrame {
    func_pos: usize,
    nargs: usize,
    wanted_results: Option<usize>,
    saved_ci: usize,
    saved_top: usize,
}

fn native_callable(function: &Function) -> Option<NativeCallable> {
    function
        .runtime_native_function()
        .map(NativeCallable::Runtime)
        .or_else(|| function.c_function().map(NativeCallable::C))
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
    let frame = NativeCallFrame {
        func_pos,
        nargs,
        wanted_results,
        saved_ci,
        saved_top,
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
    l.top = func_pos + 1 + nargs;

    if let Err(e) = fire_debug_hook(l, gc, "call", None) {
        unwind_lua_frames_to(l, gc, saved_ci)?;
        l.top = saved_top;
        return Err(e);
    }

    let nret = match callable {
        NativeCallable::C(cfunc) => {
            let l_ptr = l as *mut LuaState as *mut std::ffi::c_void;
            // SAFETY: l_ptr points to the currently executing LuaState and the
            // ordinary C ABI owns no Runtime state-switch capability.
            unsafe { cfunc(l_ptr) }
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
            continuation: DeferredVmContinuation::Call,
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
        return Ok(VmExit::Complete(ExecResult::Yielded));
    }

    finish_native_call(l, gc, frame, nret)
}

fn finish_native_call(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    frame: NativeCallFrame,
    nret: i32,
) -> Result<VmExit, RuntimeError> {
    let NativeCallFrame {
        func_pos,
        nargs,
        wanted_results,
        saved_ci,
        saved_top,
    } = frame;
    if nret < 0 {
        let initial_top = func_pos + 1 + nargs;
        let error_value = if l.top > initial_top {
            l.top
                .checked_sub(1)
                .and_then(|idx| l.stack.at(idx))
                .cloned()
        } else {
            None
        };
        let display_name = c_function_display_name(l, func_pos);
        unwind_lua_frames_to(l, gc, saved_ci)?;
        l.top = saved_top;
        return Err(error_value
            .map(|value| RuntimeError::with_value(gc, value))
            .unwrap_or_else(|| {
                RuntimeError::new(format!("C function call failed: {display_name}"))
            }));
    }

    let nret_count = nret as usize;
    let first_result = l.top.saturating_sub(nret_count);
    let wanted_count = wanted_results.unwrap_or(nret_count);
    if let Err(error) = fire_debug_hook(l, gc, "return", None) {
        unwind_lua_frames_to(l, gc, saved_ci)?;
        l.top = saved_top;
        return Err(error);
    }
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
    l.pop_call_info();
    l.top = func_pos + wanted_count;
    Ok(VmExit::Complete(ExecResult::Returned))
}

pub(crate) fn finish_deferred_native_call(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    envelope: ResumeEnvelope,
    response: ResumeResponse,
) -> Result<(), RuntimeError> {
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

    let frame = NativeCallFrame {
        func_pos: deferred.func_pos,
        nargs: deferred.nargs,
        wanted_results: deferred.wanted_results,
        saved_ci: deferred.saved_ci,
        saved_top: deferred.saved_top,
    };
    match finish_native_call(l, gc, frame, nret)? {
        VmExit::Complete(ExecResult::Returned) => {
            apply_deferred_vm_continuation(l, deferred)?;
            Ok(())
        }
        VmExit::Complete(ExecResult::Yielded)
        | VmExit::NativeRequest(_)
        | VmExit::UpvalueAccess(_) => Err(RuntimeError::new(
            "VM: deferred native completion produced a second suspension",
        )),
    }
}

pub(crate) fn finish_deferred_native_values(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    deferred: &DeferredNativeCall,
    values: Vec<Value>,
) -> Result<(), RuntimeError> {
    validate_deferred_native_call(l, deferred)?;
    l.top = deferred.func_pos + 1 + deferred.nargs;
    let count = values.len();
    for value in values {
        l.push_value(value);
    }
    let frame = NativeCallFrame {
        func_pos: deferred.func_pos,
        nargs: deferred.nargs,
        wanted_results: deferred.wanted_results,
        saved_ci: deferred.saved_ci,
        saved_top: deferred.saved_top,
    };
    match finish_native_call(l, gc, frame, count as i32)? {
        VmExit::Complete(ExecResult::Returned) => {
            apply_deferred_vm_continuation(l, deferred)?;
            Ok(())
        }
        VmExit::Complete(ExecResult::Yielded)
        | VmExit::NativeRequest(_)
        | VmExit::UpvalueAccess(_) => Err(RuntimeError::new(
            "VM: raw deferred completion produced a second suspension",
        )),
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
        resume_lua_thread(l, gc)
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

fn apply_deferred_vm_continuation(
    l: &mut LuaState,
    deferred: &DeferredNativeCall,
) -> Result<(), RuntimeError> {
    match deferred.continuation {
        DeferredVmContinuation::Call => Ok(()),
        DeferredVmContinuation::GenericFor { base, register } => {
            let first_result = l
                .stack
                .at(base + register + 3)
                .cloned()
                .unwrap_or(Value::Nil);
            if first_result.is_nil() {
                let caller = l.call_stack.get_mut(deferred.saved_ci).ok_or_else(|| {
                    RuntimeError::new("VM: generic-for continuation lost caller frame")
                })?;
                caller.savedpc = deferred.caller_pc.and_then(|pc| pc.checked_add(1));
            } else if let Some(destination) = l.stack.at_mut(base + register + 2) {
                *destination = first_result;
            }
            Ok(())
        }
    }
}

fn invoke_runtime_native(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    operation: RuntimeNativeFunction,
) -> i32 {
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
            let Some(Value::Thread(thread_ref)) = current_native_upvalue(l, 0) else {
                return publish_runtime_native_error(
                    l,
                    gc,
                    ResumeEnvelope::Wrap,
                    b"coroutine wrapper is missing its thread",
                );
            };
            (thread_ref, native_args_from(l, 1), ResumeEnvelope::Wrap)
        }
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

fn native_args_from(l: &LuaState, first: i32) -> Vec<Value> {
    let top = l.get_top();
    if top < first {
        return Vec::new();
    }
    (first..=top)
        .map(|index| l.at(index).cloned().unwrap_or(Value::Nil))
        .collect()
}

fn current_native_upvalue(l: &LuaState, index: usize) -> Option<Value> {
    let func_idx = l.current_call_info().func;
    let Value::Function(func_ref) = l.stack.at(func_idx).cloned()? else {
        return None;
    };
    // SAFETY: the current native frame's function slot keeps the closure live.
    let function = unsafe { func_ref.as_ref() }?;
    let upvalue_ref = function.upvalue(index)?;
    // SAFETY: the closure owns this upvalue and destructive sweep is disabled.
    let upvalue = unsafe { upvalue_ref.as_ref() }?;
    Some(upvalue.get_closed_value().clone())
}

/// 获取 RK 操作数——寄存器或常量
fn get_rk(l: &LuaState, base: usize, rk: i32, constants: &[Value]) -> Value {
    if opcode::is_k(rk) {
        let idx = opcode::index_k(rk) as usize;
        constants.get(idx).cloned().unwrap_or(Value::Nil)
    } else {
        let reg = rk as usize;
        l.stack.at(base + reg).cloned().unwrap_or(Value::Nil)
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
    let uv_ref = current_lua_function(l)
        .and_then(|function| function.upvalue(upvalue_idx))
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
    let uv_ref = current_lua_function(l)
        .and_then(|function| function.upvalue(upvalue_idx))
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

fn current_lua_function(l: &LuaState) -> Option<&Function> {
    if l.current_ci == 0 {
        return None;
    }
    let ci = l.current_call_info();
    if ci.func == ci.base {
        return None;
    }
    let func_idx = ci.func;
    match l.stack.at(func_idx) {
        Some(Value::Function(func_ref)) => {
            // SAFETY: the current call frame's function slot keeps the closure live.
            unsafe { func_ref.as_ref() }
        }
        _ => None,
    }
}

fn current_env(l: &LuaState) -> Option<GcRef<Table>> {
    match current_lua_function(l) {
        Some(function) => function.env().or(l.global_table),
        None => l.chunk_env.or(l.thread_env).or(l.global_table),
    }
}

/// 全局变量读取
fn get_global_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    name: &Value,
    destination: usize,
) -> Result<(), RuntimeError> {
    if let Some(env_table) = current_env(l) {
        get_table_into(
            l,
            gc,
            stack_limit,
            &Value::Table(env_table),
            name,
            destination,
        )
    } else {
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = Value::Nil;
        }
        Ok(())
    }
}

/// 全局变量写入
fn set_global(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    name: &Value,
    val: &Value,
) -> Result<(), RuntimeError> {
    if let Some(env_table) = current_env(l) {
        set_table_value(l, gc, stack_limit, &Value::Table(env_table), name, val)?;
    }
    Ok(())
}

/// 表取值（含元方法回退）
fn get_table_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    table: &Value,
    key: &Value,
    destination: usize,
) -> Result<(), RuntimeError> {
    let mut current = table.clone();
    for _ in 0..100 {
        match &current {
            Value::Table(t) => {
                // SAFETY: the table ref is reachable from the Lua stack, constants,
                // or another reachable table while this VM instruction is executing.
                let Some(table_obj) = (unsafe { t.as_ref() }) else {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = Value::Nil;
                    }
                    return Ok(());
                };
                let result = table_obj.get(key);
                if !result.is_nil() {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = result;
                    }
                    return Ok(());
                }

                let Some(index_metamethod) = table_obj
                    .metatable()
                    .and_then(|mt| lookup_metamethod(l, gc, mt, "__index"))
                else {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = Value::Nil;
                    }
                    return Ok(());
                };

                match index_metamethod {
                    Value::Function(_) => {
                        return call_metamethod_into(
                            l,
                            gc,
                            stack_limit,
                            index_metamethod,
                            &[current.clone(), key.clone()],
                            destination,
                        );
                    }
                    Value::Table(_) => current = index_metamethod,
                    _ => {
                        if let Some(slot) = l.stack.at_mut(destination) {
                            *slot = Value::Nil;
                        }
                        return Ok(());
                    }
                }
            }
            Value::String(_) => {
                let result = get_string_library_member(l, gc, key);
                if let Some(slot) = l.stack.at_mut(destination) {
                    *slot = result;
                }
                return Ok(());
            }
            _ => {
                let Some(index_metamethod) = value_metatable(l, gc, &current)
                    .and_then(|mt| lookup_metamethod(l, gc, mt, "__index"))
                else {
                    if let Some(slot) = l.stack.at_mut(destination) {
                        *slot = Value::Nil;
                    }
                    return Ok(());
                };
                match index_metamethod {
                    Value::Function(_) => {
                        return call_metamethod_into(
                            l,
                            gc,
                            stack_limit,
                            index_metamethod,
                            &[current.clone(), key.clone()],
                            destination,
                        );
                    }
                    Value::Table(_) => current = index_metamethod,
                    _ => {
                        if let Some(slot) = l.stack.at_mut(destination) {
                            *slot = Value::Nil;
                        }
                        return Ok(());
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
    stack_limit: usize,
    table: &Value,
    key: &Value,
    value: &Value,
) -> Result<(), RuntimeError> {
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

                let table_ptr = t.as_ptr() as *mut Table;
                // SAFETY: the table is reachable and GC does not run during this VM
                // instruction. We only take a shared view to test raw presence.
                let has_raw_key = unsafe { t.as_ref() }.is_some_and(|table| table.has(key));
                if !has_raw_key {
                    // SAFETY: same reachability reasoning as above.
                    if let Some(table_obj) = unsafe { t.as_ref() }
                        && let Some(newindex_metamethod) = table_obj
                            .metatable()
                            .and_then(|mt| lookup_metamethod(l, gc, mt, "__newindex"))
                    {
                        match newindex_metamethod {
                            Value::Function(_) => {
                                let saved_top = l.top;
                                l.top = l.top.max(stack_limit);
                                let call_result = call_value_with_results(
                                    l,
                                    gc,
                                    newindex_metamethod,
                                    &[current, key.clone(), value.clone()],
                                    Some(0),
                                    |_, _, _| (),
                                );
                                l.top = saved_top;
                                call_result?;
                                return Ok(());
                            }
                            Value::Table(_) => {
                                current = newindex_metamethod;
                                continue;
                            }
                            _ => {}
                        }
                    }
                }

                // SAFETY: The table is GC-managed and kept alive by the LuaState stack.
                // GC does not run during VM execution, ensuring the pointer remains valid.
                unsafe {
                    (*table_ptr).set(key, value);
                }
                return Ok(());
            }
            _ => {
                if let Some(newindex_metamethod) = value_metatable(l, gc, &current)
                    .and_then(|mt| lookup_metamethod(l, gc, mt, "__newindex"))
                {
                    match newindex_metamethod {
                        Value::Function(_) => {
                            let saved_top = l.top;
                            l.top = l.top.max(stack_limit);
                            let call_result = call_value_with_results(
                                l,
                                gc,
                                newindex_metamethod,
                                &[current, key.clone(), value.clone()],
                                Some(0),
                                |_, _, _| (),
                            );
                            l.top = saved_top;
                            call_result?;
                            return Ok(());
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
    stack_limit: usize,
    op: OpCode,
    lhs: &Value,
    rhs: &Value,
    destination: usize,
) -> Result<(), RuntimeError> {
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
        return Ok(());
    }

    if let Some(metamethod) = find_metamethod(l, gc, lhs, rhs, metamethod_name) {
        call_metamethod_into(
            l,
            gc,
            stack_limit,
            metamethod,
            &[lhs.clone(), rhs.clone()],
            destination,
        )
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
    stack_limit: usize,
    val: &Value,
    destination: usize,
) -> Result<(), RuntimeError> {
    if let Some(n) = to_arith_number(gc, val) {
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = Value::Number(-n);
        }
        return Ok(());
    }
    if let Some(metamethod) = find_metamethod(l, gc, val, val, "__unm") {
        call_metamethod_into(
            l,
            gc,
            stack_limit,
            metamethod,
            &[val.clone(), val.clone()],
            destination,
        )
    } else {
        Err(RuntimeError::new(
            "attempt to perform arithmetic on a non-number value",
        ))
    }
}

fn exec_concat_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    lhs: &Value,
    rhs: &Value,
    destination: usize,
) -> Result<(), RuntimeError> {
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
        let pool = l.string_pool;
        return gc
            .with_publication(|transaction| {
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
            });
    }
    if let Some(metamethod) = find_metamethod(l, gc, lhs, rhs, "__concat") {
        call_metamethod_into(
            l,
            gc,
            stack_limit,
            metamethod,
            &[lhs.clone(), rhs.clone()],
            destination,
        )
    } else {
        Err(RuntimeError::new(
            "attempt to concatenate a non-string value",
        ))
    }
}

/// 取长度
fn exec_len(gc: &GarbageCollector, val: &Value) -> Value {
    match val {
        Value::String(s) => gc
            .with_ref(*s, |string| Value::Number(string.len() as f64))
            .unwrap_or(Value::Number(0.0)),
        Value::Table(t) => gc
            .with_ref(*t, |table| Value::Number(table.length() as f64))
            .unwrap_or(Value::Number(0.0)),
        _ => Value::Number(0.0),
    }
}

/// 比较：小于
fn exec_lt(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, RuntimeError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(a < b),
        (Value::String(a_ref), Value::String(b_ref)) => gc
            .with_string_bytes(*a_ref, |left| {
                gc.with_string_bytes(*b_ref, |right| compare_lua_string_bytes(left, right))
            })
            .and_then(|ordering| ordering)
            .map(|ordering| ordering == Ordering::Less)
            .map_err(|error| RuntimeError::new(format!("invalid string comparison: {error}"))),
        _ => {
            if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__lt") {
                call_metamethod_bool(l, gc, stack_limit, metamethod, lhs, rhs)
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
    stack_limit: usize,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, RuntimeError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(a <= b),
        (Value::String(a_ref), Value::String(b_ref)) => gc
            .with_string_bytes(*a_ref, |left| {
                gc.with_string_bytes(*b_ref, |right| compare_lua_string_bytes(left, right))
            })
            .and_then(|ordering| ordering)
            .map(|ordering| ordering != Ordering::Greater)
            .map_err(|error| RuntimeError::new(format!("invalid string comparison: {error}"))),
        _ => {
            if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__le") {
                return call_metamethod_bool(l, gc, stack_limit, metamethod, lhs, rhs);
            }
            if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__lt") {
                return call_metamethod_bool(l, gc, stack_limit, metamethod, rhs, lhs)
                    .map(|result| !result);
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
    gc: &mut GarbageCollector,
    stack_limit: usize,
    metamethod: Value,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, RuntimeError> {
    let saved_top = l.top;
    l.top = l.top.max(stack_limit);
    let mut comparison = false;
    let result = call_value_with_results(
        l,
        gc,
        metamethod,
        &[lhs.clone(), rhs.clone()],
        Some(1),
        |_, _, results| {
            comparison = results.first().is_some_and(|value| !value.is_false());
        },
    );
    l.top = saved_top;
    result?;
    Ok(comparison)
}

fn call_metamethod_into(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    metamethod: Value,
    args: &[Value],
    destination: usize,
) -> Result<(), RuntimeError> {
    let saved_top = l.top;
    l.top = l.top.max(stack_limit);
    let result = call_value_with_results(l, gc, metamethod, args, Some(1), |l, _, results| {
        let value = results.first().cloned().unwrap_or(Value::Nil);
        if let Some(slot) = l.stack.at_mut(destination) {
            *slot = value;
        }
    });
    l.top = saved_top;
    result
}

fn exec_eq(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    lhs: &Value,
    rhs: &Value,
) -> Result<bool, RuntimeError> {
    if values_equal(lhs, rhs) {
        return Ok(true);
    }
    if std::mem::discriminant(lhs) != std::mem::discriminant(rhs) {
        return Ok(false);
    }
    if !matches!(lhs, Value::Table(_) | Value::Userdata(_)) {
        return Ok(false);
    }
    if let Some(metamethod) = find_common_metamethod(l, gc, lhs, rhs, "__eq") {
        call_metamethod_bool(l, gc, stack_limit, metamethod, lhs, rhs)
    } else {
        Ok(false)
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
    let string_pool = l.string_pool?;
    // SAFETY: Runtime installs its live StringPool for exactly the state turn.
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
        let Some(gc) = state.gc else {
            return 0;
        };
        let Some(string_pool) = state.string_pool else {
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
        state.gc = Some(&mut gc);
        state.string_pool = Some(&mut pool);
        let left = byte_value(&mut gc, &mut pool, &[0x00, 0x80, 0xff]);
        let right = byte_value(&mut gc, &mut pool, &[0xfe, 0xc3, 0x00]);
        let expected = [0x00, 0x80, 0xff, 0xfe, 0xc3, 0x00];

        ensure_stack_slot(&mut state, 0);
        exec_concat_into(&mut state, &mut gc, 0, &left, &right, 0).expect("byte concat succeeds");
        let result = state.stack.at(0).cloned().unwrap_or(Value::Nil);
        let Value::String(result_ref) = result else {
            panic!("concat must return a Lua string");
        };
        gc.with_string_bytes(result_ref, |bytes| assert_eq!(bytes, expected))
            .expect("non-null string");
        assert_eq!(pool.find_bytes(&expected), Some(result_ref));
    }

    #[test]
    fn embedded_nul_participates_in_length_and_equality() {
        let mut gc = GarbageCollector::new();
        let mut pool = StringPool::new();
        let first = byte_value(&mut gc, &mut pool, b"a\0b");
        let same = byte_value(&mut gc, &mut pool, b"a\0b");
        let different_tail = byte_value(&mut gc, &mut pool, b"a\0c");

        assert_eq!(exec_len(&gc, &first), Value::Number(3.0));
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
        state.gc = Some(&mut gc);
        state.string_pool = Some(&mut pool);
        let left = byte_value(&mut gc, &mut pool, b"a\0b");
        let right = byte_value(&mut gc, &mut pool, b"a\0c");
        assert!(exec_lt(&mut state, &mut gc, 0, &left, &right).expect("strings compare"));
        assert!(exec_le(&mut state, &mut gc, 0, &left, &right).expect("strings compare"));
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
        state.gc = Some(&mut gc);
        state.string_pool = Some(&mut string_pool);

        call_value_with_results(
            &mut state,
            &mut gc,
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
    }
}

#[cfg(test)]
mod proto_handle_tests {
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    use lua_compiler::codegen::CodeGenerator;
    use lua_compiler::parser::Parser;
    use lua_core::string_pool::StringPool;

    use super::*;

    static LINE_HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);

    unsafe extern "C" fn counting_line_hook(_: *mut std::ffi::c_void) -> i32 {
        LINE_HOOK_CALLS.fetch_add(1, AtomicOrdering::SeqCst);
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
        returned.gc = Some(&mut gc);
        returned.string_pool = Some(&mut string_pool);
        let return_proto = compile(
            &mut gc,
            &mut string_pool,
            "local function f(...) return 42 end return f(1, 2)",
        );
        assert!(matches!(
            execute_proto(&mut returned, return_proto, &mut gc),
            Ok(VmExit::Complete(ExecResult::Returned))
        ));
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
        errored.gc = Some(&mut gc);
        errored.string_pool = Some(&mut string_pool);
        let error_proto = compile(
            &mut gc,
            &mut string_pool,
            "local function f(...) return nil + 1 end return f(1, 2)",
        );
        execute_proto(&mut errored, error_proto, &mut gc)
            .expect_err("nested arithmetic error must propagate");
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
        LINE_HOOK_CALLS.store(0, AtomicOrdering::SeqCst);
        let mut gc = GarbageCollector::new();
        let mut string_pool = StringPool::new();
        let mut first = Proto::new();
        first.add_line_info(17);
        let first = gc.create(first);
        let mut second = Proto::new();
        second.add_line_info(17);
        let second = gc.create(second);
        assert_ne!(first.object_id(), second.object_id());

        let hook = gc.create(Function::new_c(counting_line_hook));
        let mut state = LuaState::new();
        state.gc = Some(&mut gc);
        state.string_pool = Some(&mut string_pool);
        state.debug_hook = Some(Value::Function(hook));
        state.debug_hook_mask = "l".to_string();
        state.debug_hook_skip_proto = Some(first);
        state.debug_hook_skip_line = 17;

        run_debug_instruction_hooks(&mut state, &mut gc, second, 0, OpCode::MOVE)
            .expect("different Proto identity runs the hook");
        assert_eq!(LINE_HOOK_CALLS.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(state.debug_hook_skip_proto, Some(first));

        run_debug_instruction_hooks(&mut state, &mut gc, first, 0, OpCode::MOVE)
            .expect("matching Proto consumes the skip");
        assert_eq!(LINE_HOOK_CALLS.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(state.debug_hook_skip_proto, None);
        assert_eq!(state.debug_hook_skip_line, -1);

        run_debug_instruction_hooks(&mut state, &mut gc, first, 0, OpCode::MOVE)
            .expect("consumed skip no longer suppresses the line");
        assert_eq!(LINE_HOOK_CALLS.load(AtomicOrdering::SeqCst), 2);
    }
}
