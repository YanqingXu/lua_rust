//! Lua 虚拟机执行引擎
#![allow(clippy::collapsible_if, clippy::collapsible_match)]
//!
//! 基于寄存器的字节码解释器，实现全部 38 条 Lua 5.1 指令。
//! 使用 Rust match 进行指令分发（编译器生成跳转表，性能对标 C++ switch）。
//!
//! C++ 参考: `lua_cpp/src/vm/vm.cpp`, `vm_handlers/`

use lua_compiler::opcode::{self, OpCode};
use lua_core::function::{CFunction, Function};
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::header::GcObjectHeader;
use lua_core::gc_string::GcString;
use lua_core::proto::{Proto, VARARG_NEEDSARG};
use lua_core::table::Table;
use lua_core::upvalue::Upvalue;
use lua_core::value::Value;

use crate::state::{LUA_MULTRET, LuaState, Stack, ThreadStatus};

/// 执行结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecResult {
    Returned,
    Yielded,
}

/// 最大嵌套调用深度
const MAX_CALLS: i32 = 512;
const MAX_STRING_LENGTH: usize = 64 * 1024 * 1024;

/// 虚拟机主执行循环
///
/// 对标 C++ `VM::executeProto()`。
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
    proto: &Proto,
    gc: &mut GarbageCollector,
) -> Result<ExecResult, RuntimeError> {
    if l.nccalls > MAX_CALLS {
        return Err(RuntimeError::new(
            "VM: stack overflow (too many nested calls)",
        ));
    }
    l.gc = Some(gc as *mut GarbageCollector);

    let mut active_proto = proto;
    let _nresults = l.current_call_info().nresults;
    let resume_from_yield = l.status == ThreadStatus::Yield;
    let mut pc: usize = if resume_from_yield {
        l.status = ThreadStatus::Ok;
        l.current_call_info_mut().proto = Some(active_proto as *const Proto);
        l.current_call_info()
            .savedpc
            .map(|savedpc| savedpc + 1)
            .unwrap_or(0)
    } else {
        let ci = l.current_call_info_mut();
        ci.savedpc = Some(0); // start at PC 0
        ci.proto = Some(active_proto as *const Proto);
        0
    };

    // Ensure stack has enough space for this function's registers.
    // Proto::max_stack_size() gives the number of register slots needed.
    let stack_needed = l.current_call_info().base + active_proto.max_stack_size() as usize;
    if l.stack.size() < stack_needed {
        l.stack.set_top(stack_needed);
    }
    if l.current_call_info().top < stack_needed {
        l.current_call_info_mut().top = stack_needed;
    }

    // 主解释循环
    loop {
        l.current_proto = Some(active_proto as *const Proto);
        let code = active_proto.code();
        if pc >= code.len() {
            break;
        }
        let constants = active_proto.constants();
        let inst = code[pc];
        let op = opcode::get_opcode(inst);
        let base_idx = l.current_call_info().base;
        l.current_call_info_mut().savedpc = Some(pc);
        run_debug_instruction_hooks(l, gc, active_proto, pc, op)?;
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
                let val = get_upvalue(l, b)?;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = val;
                }
            }

            OpCode::GETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // R(A) := _G[K(Bx)]
                let name = constants.get(bx).cloned().unwrap_or(Value::Nil);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let val = get_global(l, gc, stack_limit, &name)?;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = val;
                }
            }

            OpCode::GETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let table = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                if should_error_index(l, &table) {
                    return Err(runtime_error_at(
                        active_proto,
                        pc,
                        index_error_message(active_proto, pc, b, constants, &table),
                    ));
                }
                let result = get_table(l, gc, stack_limit, &table, &key)?;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
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
                set_upvalue(l, b, &val)?;
            }

            OpCode::SETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let key = get_rk(l, base_idx, b, constants);
                let value = get_rk(l, base_idx, c, constants);
                let table_val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                if should_error_newindex(l, &table_val) {
                    return Err(runtime_error_at(
                        active_proto,
                        pc,
                        index_error_message(active_proto, pc, a, constants, &table_val),
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
                let table = Table::with_capacity(b as usize, c as usize);
                let table_ref: GcRef<Table> = gc.create(table);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Table(table_ref);
                }
            }

            OpCode::SELF => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let obj = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, base_idx, c, constants);
                if should_error_index(l, &obj) {
                    return Err(runtime_error_at(
                        active_proto,
                        pc,
                        index_error_message(active_proto, pc, b, constants, &obj),
                    ));
                }
                // R(A+1) = R(B)
                if let Some(dst) = l.stack.at_mut(base_idx + a + 1) {
                    *dst = obj.clone();
                }
                // R(A) = R(B)[RK(C)]
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let result = get_table(l, gc, stack_limit, &obj, &key)?;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
            }

            // ── 算术运算 (6) ─────────────────────────────────
            OpCode::ADD | OpCode::SUB | OpCode::MUL | OpCode::DIV | OpCode::MOD | OpCode::POW => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b, constants);
                let rhs = get_rk(l, base_idx, c, constants);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let result = match exec_arith(l, gc, stack_limit, op, &lhs, &rhs) {
                    Ok(result) => result,
                    Err(err)
                        if err.message == "attempt to perform arithmetic on a non-number value" =>
                    {
                        return Err(RuntimeError::new(arith_error_message(
                            active_proto,
                            pc,
                            b,
                            c,
                            constants,
                            &lhs,
                            &rhs,
                        )));
                    }
                    Err(err) => return Err(err),
                };
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
            }

            // ── 一元运算 (3) ─────────────────────────────────
            OpCode::UNM => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let val = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let stack_limit = base_idx + active_proto.max_stack_size() as usize;
                let result = match exec_unm(l, gc, stack_limit, &val) {
                    Ok(result) => result,
                    Err(err)
                        if err.message == "attempt to perform arithmetic on a non-number value" =>
                    {
                        return Err(RuntimeError::new(unm_error_message(
                            active_proto,
                            pc,
                            b,
                            constants,
                            &val,
                        )));
                    }
                    Err(err) => return Err(err),
                };
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
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
                let len = exec_len(&val);
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
                let mut result = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                for i in (b + 1)..=c {
                    let rhs = l.stack.at(base_idx + i).cloned().unwrap_or(Value::Nil);
                    result = exec_concat(l, gc, stack_limit, &result, &rhs)?;
                }
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
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
                                // SAFETY: callee_proto_ref is kept alive by the Function
                                // GC object which is on the stack during execution
                                let callee_proto = unsafe { callee_proto_ref.as_ref() };
                                if let Some(callee_proto) = callee_proto {
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
                                    ci.proto = Some(callee_proto as *const Proto);
                                    ci.tailcalls = 0;

                                    if let Err(e) = fire_debug_hook(l, gc, "call", None) {
                                        l.current_ci = saved_ci;
                                        return Err(e);
                                    }

                                    // Recursively execute the called function
                                    match execute_nested_proto_at(
                                        l,
                                        active_proto,
                                        pc,
                                        callee_proto,
                                        gc,
                                    ) {
                                        Ok(ExecResult::Returned) => {
                                            // Results already placed by RETURN handler
                                        }
                                        Ok(ExecResult::Yielded) => {
                                            return Ok(ExecResult::Yielded);
                                        }
                                        Err(e) => {
                                            // Restore call frame and propagate error
                                            let close_base = l.current_call_info().base;
                                            l.close_upvalues(close_base);
                                            l.current_ci = saved_ci;
                                            return Err(e);
                                        }
                                    }

                                    if l.current_ci != saved_ci {
                                        return Err(RuntimeError::new("VM: call frame imbalance"));
                                    }
                                    pc += 1;
                                    continue;
                                }
                            }
                            return Err(RuntimeError::new("Lua function has no proto"));
                        } else if let Some(cfunc) = func_obj.c_function() {
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
                            match call_c_function(
                                l,
                                gc,
                                func_pos,
                                actual_nargs,
                                wanted_results,
                                cfunc,
                            ) {
                                Ok(ExecResult::Yielded) => return Ok(ExecResult::Yielded),
                                Ok(ExecResult::Returned) => {}
                                Err(err) if err.message.starts_with("bad argument") => {
                                    return Err(runtime_error_at(active_proto, pc, err.message));
                                }
                                Err(err) => return Err(err),
                            }
                        }
                    }
                } else if let Some(metamethod) = find_metamethod(l, &func, &func, "__call") {
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
                    let detail = describe_register(active_proto, pc, a, constants, 8)
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
                    && let Some(metamethod) = find_metamethod(l, &func, &func, "__call")
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
                    l.close_upvalues(ci.base);
                    if available > 0 {
                        ensure_stack_slot(l, ci.func + available - 1);
                    }
                    for (i, result) in results.into_iter().enumerate() {
                        if let Some(dst) = l.stack.at_mut(ci.func + i) {
                            *dst = result;
                        }
                    }
                    l.top = ci.func + available;
                    return Ok(ExecResult::Returned);
                }

                if let Value::Function(func_ref) = func {
                    // SAFETY: GC is not running during VM execution; GcRef remains valid
                    if let Some(func_obj) = unsafe { func_ref.as_ref() } {
                        if func_obj.is_lua_function() {
                            if let Some(tail_proto_ref) = func_obj.proto() {
                                let tail_proto_ptr = tail_proto_ref.as_ptr();
                                if !tail_proto_ptr.is_null() {
                                    // SAFETY: tail_proto_ref is kept alive by the Function GC object,
                                    // and GC does not run while the VM is executing this frame.
                                    let tail_proto = unsafe { &*tail_proto_ptr };
                                    let args: Vec<Value> = (0..actual_nargs)
                                        .map(|i| {
                                            l.stack
                                                .at(func_pos + 1 + i)
                                                .cloned()
                                                .unwrap_or(Value::Nil)
                                        })
                                        .collect();
                                    l.close_upvalues(base_idx);
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
                                    ci.proto = Some(tail_proto as *const Proto);
                                    ci.tailcalls += 1;

                                    if l.stack.size() < new_top {
                                        l.stack.set_top(new_top);
                                    }
                                    fire_debug_hook(l, gc, "call", None)?;
                                    active_proto = tail_proto;
                                    pc = 0;
                                    continue;
                                }
                            }
                        } else if let Some(cfunc) = func_obj.c_function() {
                            if call_c_function(l, gc, func_pos, actual_nargs, None, cfunc)?
                                == ExecResult::Yielded
                            {
                                return Ok(ExecResult::Yielded);
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

                            l.close_upvalues(ci.base);
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
                            return Ok(ExecResult::Returned);
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

                l.close_upvalues(ci.base);

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
                return Ok(ExecResult::Returned);
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
                let step_num = as_number(&step);
                let limit_num = as_number(&limit);
                let idx_num = as_number(&idx_val) + step_num;
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
                let step_num = match to_arith_number(&step) {
                    Some(step) => step,
                    None => {
                        return Err(runtime_error_at(
                            active_proto,
                            pc,
                            "'for' step must be a number",
                        ));
                    }
                };
                if to_arith_number(&limit).is_none() {
                    return Err(runtime_error_at(
                        active_proto,
                        pc,
                        "'for' limit must be a number",
                    ));
                }
                let init_num = match to_arith_number(&init) {
                    Some(init) => init - step_num,
                    None => {
                        return Err(runtime_error_at(
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
                            if let Some(cfunc) = func_obj.c_function() {
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
                                if call_c_function(l, gc, call_pos, 2, Some(c), cfunc)?
                                    == ExecResult::Yielded
                                {
                                    return Ok(ExecResult::Yielded);
                                }
                            } else if let Some(iter_proto_ref) = func_obj.proto() {
                                // SAFETY: iter_proto_ref is kept alive by the iterator closure
                                // stored in the generic-for generator register.
                                let iter_proto =
                                    unsafe { iter_proto_ref.as_ref() }.ok_or_else(|| {
                                        RuntimeError::new("generic for: Lua iterator has no proto")
                                    })?;
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
                                ci.proto = Some(iter_proto as *const Proto);
                                ci.tailcalls = 0;

                                match execute_nested_proto_at(l, active_proto, pc, iter_proto, gc) {
                                    Ok(ExecResult::Returned) => {}
                                    Ok(ExecResult::Yielded) => return Ok(ExecResult::Yielded),
                                    Err(e) => {
                                        let close_base = l.current_call_info().base;
                                        l.close_upvalues(close_base);
                                        l.current_ci = saved_ci;
                                        return Err(e);
                                    }
                                }

                                if l.current_ci != saved_ci {
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
                l.close_upvalues(base_idx + a);
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
                    let mut func = Function::new_lua(sub_proto_ref);
                    func.set_env(current_env(l));
                    // SAFETY: sub_proto_ref is a live child proto owned by the
                    // currently executing parent proto.
                    let child_proto = unsafe { sub_proto_ref.as_ref() }
                        .ok_or_else(|| RuntimeError::new("VM: CLOSURE invalid child proto"))?;
                    let mut next_pc = pc + 1;
                    for _ in 0..child_proto.num_upvalues() {
                        let pseudo = *code.get(next_pc).ok_or_else(|| {
                            RuntimeError::new("VM: CLOSURE missing upvalue pseudo instruction")
                        })?;
                        next_pc += 1;

                        let upvalue_ref = match opcode::get_opcode(pseudo) {
                            OpCode::MOVE => {
                                let b = opcode::get_arg_b(pseudo) as usize;
                                l.find_or_create_upvalue(base_idx + b, gc)
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
                        func.add_upvalue(upvalue_ref);
                    }

                    let func_ref: GcRef<Function> = gc.create(func);
                    if let Some(dst) = l.stack.at_mut(base_idx + a) {
                        *dst = Value::Function(func_ref);
                    }
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

    Ok(ExecResult::Returned)
}

fn run_debug_instruction_hooks(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    proto: &Proto,
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
        let line = proto.line(pc);
        let should_skip_setup_line = l.debug_hook_skip_line == line
            && l.debug_hook_skip_proto == Some(proto as *const Proto);
        let repeated_line_from_jump = pc <= l.debug_hook_last_pc && line == l.debug_hook_last_line;
        if should_skip_setup_line {
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

        let Some(proto_ptr) = frame_proto_for_gc(l, ci) else {
            continue;
        };
        // SAFETY: proto pointers are installed by the VM while their frames are live.
        let proto = unsafe { &*proto_ptr };
        let pc = ci.savedpc.unwrap_or(0) as i32;
        for idx in 0..proto.loc_var_count() {
            let loc = proto.loc_var(idx);
            if loc.startpc <= pc && pc < loc.endpc && loc.reg >= 0 {
                let stack_index = ci.base + loc.reg as usize;
                if let Some(value) = l.stack.at(stack_index) {
                    gc.mark_value(value);
                }
            }
        }
    }
}

fn frame_proto_for_gc(l: &LuaState, ci: &crate::state::CallInfo) -> Option<*const Proto> {
    if let Some(proto) = ci.proto {
        return Some(proto);
    }
    let Value::Function(func_ref) = l.stack.at(ci.func).cloned().unwrap_or(Value::Nil) else {
        return l.current_proto;
    };
    // SAFETY: function refs read from active stack slots stay live during this call.
    let func = unsafe { func_ref.as_ref() }?;
    if !func.is_lua_function() {
        return None;
    }
    func.proto().map(|proto| proto.as_ptr() as *const Proto)
}

fn mark_open_upvalues(l: &LuaState, gc: &mut GarbageCollector) {
    let mut current = l.open_upvalues;
    while let Some(upvalue_ref) = current {
        // SAFETY: open_upvalues only contains live Upvalue refs allocated by GC.
        let Some(upvalue) = (unsafe { upvalue_ref.as_ref() }) else {
            break;
        };
        // SAFETY: upvalue_ref points to a GC-managed Upvalue object.
        unsafe {
            gc.mark_object(upvalue_ref.as_ptr() as *mut GcObjectHeader);
        }
        if upvalue.is_open()
            && let Some(value) = l.stack.at(upvalue.stack_index())
        {
            gc.mark_value(value);
        }
        current = upvalue.next();
    }
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

    let event_ref = gc.create(GcString::new(event));
    let line_value = line.map_or(Value::Nil, |line| Value::Number(line as f64));
    let saved_top = l.top;
    let frame_top = l.current_call_info().top.max(saved_top);
    l.top = frame_top;
    l.debug_hook_active = true;
    let result = call_value(
        l,
        gc,
        hook,
        &[Value::String(event_ref), line_value],
        Some(0),
    );
    l.debug_hook_active = false;
    l.top = saved_top;
    result.map(|_| ())
}

fn execute_nested_proto_at(
    l: &mut LuaState,
    caller_proto: &Proto,
    caller_pc: usize,
    callee_proto: &Proto,
    gc: &mut GarbageCollector,
) -> Result<ExecResult, RuntimeError> {
    let overflow = stack_overflow_error(l, caller_proto, caller_pc);
    execute_counted_proto(l, callee_proto, gc, overflow)
}

fn execute_counted_proto(
    l: &mut LuaState,
    proto: &Proto,
    gc: &mut GarbageCollector,
    overflow: RuntimeError,
) -> Result<ExecResult, RuntimeError> {
    if l.nccalls >= MAX_CALLS {
        return Err(overflow);
    }
    l.nccalls += 1;
    let result = execute_proto(l, proto, gc);
    l.nccalls -= 1;
    result
}

/// Call a Lua value from host/stdlib code and collect its results.
///
/// This mirrors the VM CALL path but restores the caller's stack window before
/// returning, making it suitable for protected helpers such as `pcall`.
pub fn call_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    func: Value,
    args: &[Value],
    wanted_results: Option<usize>,
) -> Result<Vec<Value>, RuntimeError> {
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

    l.current_ci = saved_ci;
    l.top = saved_top;
    result
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
    // SAFETY: the function on the stack keeps its Proto alive while it runs.
    let Some(callee_proto) = (unsafe { callee_proto_ref.as_ref() }) else {
        return Err(RuntimeError::new("Lua function has invalid proto"));
    };

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
    ci.proto = Some(callee_proto as *const Proto);
    ci.tailcalls = 0;

    Ok(())
}

pub fn resume_lua_thread(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
) -> Result<ExecResult, RuntimeError> {
    l.gc = Some(gc as *mut GarbageCollector);

    loop {
        if l.current_ci == 0 {
            return Ok(ExecResult::Returned);
        }

        let proto_ref = current_lua_function(l)
            .and_then(|function| function.proto())
            .ok_or_else(|| RuntimeError::new("coroutine frame has no Lua proto"))?;
        // SAFETY: the current frame's function slot keeps the proto alive.
        let proto = unsafe { proto_ref.as_ref() }
            .ok_or_else(|| RuntimeError::new("coroutine frame has invalid proto"))?;

        l.status = ThreadStatus::Yield;
        match execute_proto(l, proto, gc)? {
            ExecResult::Yielded => return Ok(ExecResult::Yielded),
            ExecResult::Returned => {
                if l.current_ci == 0 {
                    return Ok(ExecResult::Returned);
                }
            }
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

    if let Some(cfunc) = func_obj.c_function() {
        if call_c_function(l, gc, func_pos, nargs, wanted_results, cfunc)? == ExecResult::Yielded {
            return Err(RuntimeError::new("cannot yield across pcall"));
        }
        return Ok(());
    }

    let Some(callee_proto_ref) = func_obj.proto() else {
        return Err(RuntimeError::new("Lua function has no proto"));
    };
    // SAFETY: the function on the stack keeps its Proto alive while it runs.
    let Some(callee_proto) = (unsafe { callee_proto_ref.as_ref() }) else {
        return Err(RuntimeError::new("Lua function has invalid proto"));
    };

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
    ci.proto = Some(callee_proto as *const Proto);
    ci.tailcalls = 0;

    match execute_counted_proto(l, callee_proto, gc, RuntimeError::new("stack overflow")) {
        Ok(ExecResult::Returned) => {}
        Ok(ExecResult::Yielded) => return Err(RuntimeError::new("cannot yield across pcall")),
        Err(e) => {
            let close_base = l.current_call_info().base;
            l.close_upvalues(close_base);
            l.current_ci = saved_ci;
            return Err(e);
        }
    }

    if l.current_ci != saved_ci {
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
    find_function_name_in_table(global, func_ref).unwrap_or_else(|| {
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
            if let Some(name) = find_function_name_in_table(table, func_ref) {
                // SAFETY: key is held by the global table.
                let lib_name = unsafe { lib_name_ref.as_ref() }
                    .map(|name| name.data().to_string())
                    .unwrap_or_default();
                return format!("{lib_name}.{name}");
            }
        }
        format!("function: {:p}", func_ref.as_ptr())
    })
}

fn find_function_name_in_table(table: &Table, func_ref: GcRef<Function>) -> Option<String> {
    for (key, value) in table.hash_entries() {
        if let (Value::String(name_ref), Value::Function(value_ref)) = (key, value)
            && *value_ref == func_ref
        {
            // SAFETY: key is held by the table being inspected.
            return unsafe { name_ref.as_ref() }.map(|name| name.data().to_string());
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

fn runtime_error_at(proto: &Proto, pc: usize, message: impl Into<String>) -> RuntimeError {
    let source = proto
        .source()
        .and_then(|source_ref| {
            // SAFETY: the active Proto keeps its source string alive.
            unsafe { source_ref.as_ref() }.map(|source| source.data().to_string())
        })
        .unwrap_or_else(|| "?".to_string());
    let line = proto.line(pc);
    RuntimeError::new(format!("{}:{}: {}", source, line, message.into()))
}

fn stack_overflow_error(l: &LuaState, caller_proto: &Proto, caller_pc: usize) -> RuntimeError {
    let recursive_line = caller_proto.line(caller_pc);
    let mut message = format!("?:{}: stack overflow", recursive_line);
    for _ in 0..20 {
        message.push_str(&format!("\n?:{}: in function 'y'", recursive_line));
    }
    if let Some(line) = first_non_recursive_caller_line(l) {
        message.push_str(&format!("\n?:{}: in function 'g'", line));
    } else {
        let line = caller_proto.line(caller_pc);
        message.push_str(&format!("\n?:{}: in function", line));
    }
    RuntimeError::new(message)
}

fn first_non_recursive_caller_line(l: &LuaState) -> Option<i32> {
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
        // SAFETY: the function is held by a live call frame.
        let func = unsafe { func_ref.as_ref() }?;
        let proto_ref = func.proto()?;
        // SAFETY: a Lua function keeps its proto alive.
        let proto = unsafe { proto_ref.as_ref() }?;
        return ci.savedpc.map(|pc| proto.line(pc));
    }
    None
}

fn should_error_index(l: &LuaState, value: &Value) -> bool {
    !matches!(value, Value::Table(_) | Value::String(_))
        && value_metatable(l, value)
            .and_then(|mt| lookup_metamethod(mt, "__index"))
            .is_none()
}

fn should_error_newindex(l: &LuaState, value: &Value) -> bool {
    !matches!(value, Value::Table(_))
        && value_metatable(l, value)
            .and_then(|mt| lookup_metamethod(mt, "__newindex"))
            .is_none()
}

fn index_error_message(
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    value: &Value,
) -> String {
    let type_desc = value_type_name(value);
    describe_register_for_index(proto, pc, reg, constants, 8)
        .map(|name| format!("attempt to index {name} (a {type_desc} value)"))
        .unwrap_or_else(|| format!("attempt to index a {type_desc} value"))
}

fn arith_error_message(
    proto: &Proto,
    pc: usize,
    lhs_rk: i32,
    rhs_rk: i32,
    constants: &[Value],
    lhs: &Value,
    rhs: &Value,
) -> String {
    let (bad_rk, bad_value) = if to_arith_number(lhs).is_none() {
        (lhs_rk, lhs)
    } else {
        (rhs_rk, rhs)
    };
    operand_error_message(
        proto,
        pc,
        bad_rk,
        constants,
        bad_value,
        "attempt to perform arithmetic on",
    )
}

fn unm_error_message(
    proto: &Proto,
    pc: usize,
    rk: usize,
    constants: &[Value],
    value: &Value,
) -> String {
    operand_error_message(
        proto,
        pc,
        rk as i32,
        constants,
        value,
        "attempt to perform arithmetic on",
    )
}

fn operand_error_message(
    proto: &Proto,
    pc: usize,
    rk: i32,
    constants: &[Value],
    value: &Value,
    prefix: &str,
) -> String {
    let type_desc = value_type_name(value);
    describe_operand(proto, pc, rk, constants, 8)
        .map(|name| format!("{prefix} {name} (a {type_desc} value)"))
        .unwrap_or_else(|| format!("{prefix} a non-number value"))
}

fn describe_operand(
    proto: &Proto,
    pc: usize,
    rk: i32,
    constants: &[Value],
    depth: usize,
) -> Option<String> {
    if opcode::is_k(rk) {
        None
    } else {
        describe_register(proto, pc, rk as usize, constants, depth)
    }
}

fn describe_register(
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    depth: usize,
) -> Option<String> {
    describe_register_impl(proto, pc, reg, constants, depth, true)
}

fn describe_register_for_index(
    proto: &Proto,
    pc: usize,
    reg: usize,
    constants: &[Value],
    depth: usize,
) -> Option<String> {
    describe_register_impl(proto, pc, reg, constants, depth, false)
}

fn describe_register_impl(
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

    if let Some(name) = local_name_for_reg(proto, reg, pc) {
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
                    .and_then(gc_string_data)
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
                return constant_string(constants, bx).map(|name| format!("global '{name}'"));
            }
            OpCode::GETTABLE => {
                let key = opcode::get_arg_c(inst);
                if let Some(name) = rk_string(constants, key) {
                    return Some(format!("field '{name}'"));
                }
                return None;
            }
            OpCode::SELF => {
                let key = opcode::get_arg_c(inst);
                if let Some(name) = rk_string(constants, key) {
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

fn local_name_for_reg(proto: &Proto, reg: usize, pc: usize) -> Option<String> {
    let pc = pc as i32;
    for idx in (0..proto.loc_var_count()).rev() {
        let loc = proto.loc_var(idx);
        if loc.reg == reg as i32
            && loc.startpc <= pc
            && pc < loc.endpc
            && let Some(name_ref) = loc.varname
        {
            return gc_string_data(name_ref);
        }
    }
    None
}

fn rk_string(constants: &[Value], rk: i32) -> Option<String> {
    if opcode::is_k(rk) {
        constant_string(constants, opcode::index_k(rk) as usize)
    } else {
        None
    }
}

fn constant_string(constants: &[Value], idx: usize) -> Option<String> {
    match constants.get(idx) {
        Some(Value::String(name_ref)) => gc_string_data(*name_ref),
        _ => None,
    }
}

fn gc_string_data(name_ref: GcRef<GcString>) -> Option<String> {
    // SAFETY: debug names/constants are owned by the active Proto while executing.
    unsafe { name_ref.as_ref() }.map(|name| name.data().to_string())
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

    let mut table = Table::new();
    for (idx, value) in varargs.iter().enumerate() {
        table.set(&Value::Number((idx + 1) as f64), value);
    }
    let n_key = gc.create(GcString::new("n"));
    table.set(&Value::String(n_key), &Value::Number(varargs.len() as f64));

    let table_ref = gc.create(table);
    if let Some(dst) = l.stack.at_mut(slot) {
        *dst = Value::Table(table_ref);
    }
}

fn call_c_function(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    func_pos: usize,
    nargs: usize,
    wanted_results: Option<usize>,
    cfunc: CFunction,
) -> Result<ExecResult, RuntimeError> {
    let saved_ci = l.current_ci;
    let saved_top = l.top;
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
        l.current_ci = saved_ci;
        l.top = saved_top;
        return Err(e);
    }

    let l_ptr = l as *mut LuaState as *mut std::ffi::c_void;
    // SAFETY: l_ptr points to the currently executing LuaState.
    let nret = unsafe { cfunc(l_ptr) };

    if l.status == ThreadStatus::Yield {
        l.yield_result_base = Some(func_pos);
        l.yield_wanted_results = wanted_results;
        l.pop_call_info();
        l.top = func_pos;
        return Ok(ExecResult::Yielded);
    }

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
        l.current_ci = saved_ci;
        l.top = saved_top;
        return Err(error_value
            .map(RuntimeError::with_value)
            .unwrap_or_else(|| {
                RuntimeError::new(format!(
                    "C function call failed: {}",
                    c_function_display_name(l, func_pos)
                ))
            }));
    }

    let nret_count = nret as usize;
    let first_result = l.top.saturating_sub(nret_count);
    let wanted_count = wanted_results.unwrap_or(nret_count);
    fire_debug_hook(l, gc, "return", None)?;
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
    Ok(ExecResult::Returned)
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
fn get_upvalue(l: &LuaState, upvalue_idx: usize) -> Result<Value, RuntimeError> {
    let uv_ref = current_lua_function(l)
        .and_then(|function| function.upvalue(upvalue_idx))
        .ok_or_else(|| RuntimeError::new("VM: GETUPVAL invalid upvalue index"))?;

    // SAFETY: the upvalue is kept alive by the currently executing closure.
    let uv = unsafe { uv_ref.as_ref() }
        .ok_or_else(|| RuntimeError::new("VM: GETUPVAL invalid upvalue ref"))?;
    if uv.is_open() {
        let owner_stack = uv.owner_stack();
        if owner_stack.is_null() {
            Ok(l.stack.at(uv.stack_index()).cloned().unwrap_or(Value::Nil))
        } else {
            // SAFETY: open upvalues store the Stack pointer supplied by the owning LuaState.
            let stack = unsafe { &*(owner_stack as *const Stack) };
            Ok(stack.at(uv.stack_index()).cloned().unwrap_or(Value::Nil))
        }
    } else {
        Ok(uv.get_closed_value().clone())
    }
}

/// 设置上值
fn set_upvalue(l: &mut LuaState, upvalue_idx: usize, val: &Value) -> Result<(), RuntimeError> {
    let uv_ref = current_lua_function(l)
        .and_then(|function| function.upvalue(upvalue_idx))
        .ok_or_else(|| RuntimeError::new("VM: SETUPVAL invalid upvalue index"))?;

    // SAFETY: the upvalue is kept alive by the currently executing closure.
    let uv = unsafe { &mut *(uv_ref.as_ptr() as *mut Upvalue) };
    if uv.is_open() {
        let idx = uv.stack_index();
        let owner_stack = uv.owner_stack();
        if owner_stack.is_null() {
            if let Some(slot) = l.stack.at_mut(idx) {
                *slot = val.clone();
            }
        } else {
            // SAFETY: open upvalues store the Stack pointer supplied by the owning LuaState.
            let stack = unsafe { &mut *(owner_stack as *mut Stack) };
            if let Some(slot) = stack.at_mut(idx) {
                *slot = val.clone();
            }
        }
    } else {
        uv.set_closed_value(val.clone());
    }
    Ok(())
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
fn get_global(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    name: &Value,
) -> Result<Value, RuntimeError> {
    if let Some(env_table) = current_env(l) {
        get_table(l, gc, stack_limit, &Value::Table(env_table), name)
    } else {
        Ok(Value::Nil)
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
fn get_table(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    table: &Value,
    key: &Value,
) -> Result<Value, RuntimeError> {
    let mut current = table.clone();
    for _ in 0..100 {
        match &current {
            Value::Table(t) => {
                // SAFETY: the table ref is reachable from the Lua stack, constants,
                // or another reachable table while this VM instruction is executing.
                let Some(table_obj) = (unsafe { t.as_ref() }) else {
                    return Ok(Value::Nil);
                };
                let result = table_obj.get(key);
                if !result.is_nil() {
                    return Ok(result);
                }

                let Some(index_metamethod) = table_obj
                    .metatable()
                    .and_then(|mt| lookup_metamethod(mt, "__index"))
                else {
                    return Ok(Value::Nil);
                };

                match index_metamethod {
                    Value::Function(_) => {
                        return call_metamethod_value(
                            l,
                            gc,
                            stack_limit,
                            index_metamethod,
                            &[current.clone(), key.clone()],
                        );
                    }
                    Value::Table(_) => current = index_metamethod,
                    _ => return Ok(Value::Nil),
                }
            }
            Value::String(_) => return Ok(get_string_library_member(l, key)),
            _ => {
                let Some(index_metamethod) =
                    value_metatable(l, &current).and_then(|mt| lookup_metamethod(mt, "__index"))
                else {
                    return Ok(Value::Nil);
                };
                match index_metamethod {
                    Value::Function(_) => {
                        return call_metamethod_value(
                            l,
                            gc,
                            stack_limit,
                            index_metamethod,
                            &[current.clone(), key.clone()],
                        );
                    }
                    Value::Table(_) => current = index_metamethod,
                    _ => return Ok(Value::Nil),
                }
            }
        }
    }
    Err(RuntimeError::new("'__index' chain too long"))
}

fn get_string_library_member(l: &LuaState, key: &Value) -> Value {
    let Some(global_table) = l.global_table else {
        return Value::Nil;
    };
    // SAFETY: the global table is rooted and GC does not run during VM execution.
    let Some(global) = (unsafe { global_table.as_ref() }) else {
        return Value::Nil;
    };

    for (global_key, global_value) in global.hash_entries() {
        let Value::String(name_ref) = global_key else {
            continue;
        };
        // SAFETY: keys are held by the rooted global table.
        let Some(name) = (unsafe { name_ref.as_ref() }) else {
            continue;
        };
        if name.data() == "string"
            && let Value::Table(string_table_ref) = global_value
            // SAFETY: string library table is reachable from the global table.
            && let Some(string_table) = unsafe { string_table_ref.as_ref() }
        {
            return string_table.get(key);
        }
    }

    Value::Nil
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
                            .and_then(|mt| lookup_metamethod(mt, "__newindex"))
                    {
                        match newindex_metamethod {
                            Value::Function(_) => {
                                let saved_top = l.top;
                                l.top = l.top.max(stack_limit);
                                let call_result = call_value(
                                    l,
                                    gc,
                                    newindex_metamethod,
                                    &[current, key.clone(), value.clone()],
                                    Some(0),
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
                if let Some(newindex_metamethod) =
                    value_metatable(l, &current).and_then(|mt| lookup_metamethod(mt, "__newindex"))
                {
                    match newindex_metamethod {
                        Value::Function(_) => {
                            let saved_top = l.top;
                            l.top = l.top.max(stack_limit);
                            let call_result = call_value(
                                l,
                                gc,
                                newindex_metamethod,
                                &[current, key.clone(), value.clone()],
                                Some(0),
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
fn exec_arith(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    op: OpCode,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeError> {
    let metamethod_name = match op {
        OpCode::ADD => "__add",
        OpCode::SUB => "__sub",
        OpCode::MUL => "__mul",
        OpCode::DIV => "__div",
        OpCode::MOD => "__mod",
        OpCode::POW => "__pow",
        _ => return Err(RuntimeError::new("unknown arithmetic operation")),
    };

    if let (Some(a), Some(b)) = (to_arith_number(lhs), to_arith_number(rhs)) {
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
        return Ok(Value::Number(result));
    }

    if let Some(metamethod) = find_metamethod(l, lhs, rhs, metamethod_name) {
        call_metamethod_value(l, gc, stack_limit, metamethod, &[lhs.clone(), rhs.clone()])
    } else {
        Err(RuntimeError::new(
            "attempt to perform arithmetic on a non-number value",
        ))
    }
}

/// 取负
fn exec_unm(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    val: &Value,
) -> Result<Value, RuntimeError> {
    if let Some(n) = to_arith_number(val) {
        return Ok(Value::Number(-n));
    }
    if let Some(metamethod) = find_metamethod(l, val, val, "__unm") {
        call_metamethod_value(l, gc, stack_limit, metamethod, &[val.clone(), val.clone()])
    } else {
        Err(RuntimeError::new(
            "attempt to perform arithmetic on a non-number value",
        ))
    }
}

fn exec_concat(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, RuntimeError> {
    if let (Some(left), Some(right)) = (to_concat_string(lhs), to_concat_string(rhs)) {
        let len = left
            .len()
            .checked_add(right.len())
            .ok_or_else(|| RuntimeError::new("string length overflow"))?;
        if len > MAX_STRING_LENGTH {
            return Err(RuntimeError::new("string length overflow"));
        }
        return Ok(Value::String(gc.create(GcString::new(&(left + &right)))));
    }
    if let Some(metamethod) = find_metamethod(l, lhs, rhs, "__concat") {
        call_metamethod_value(l, gc, stack_limit, metamethod, &[lhs.clone(), rhs.clone()])
    } else {
        Err(RuntimeError::new(
            "attempt to concatenate a non-string value",
        ))
    }
}

/// 取长度
fn exec_len(val: &Value) -> Value {
    match val {
        Value::String(s) => {
            // SAFETY: GC does not run during VM execution; the GcString is alive
            // as long as it's reachable from the LuaState stack or proto constants.
            if let Some(gc_str) = unsafe { s.as_ref() } {
                Value::Number(gc_str.len() as f64)
            } else {
                Value::Number(0.0)
            }
        }
        Value::Table(t) => {
            // SAFETY: GC does not run during VM execution; the table is alive
            // as long as it's reachable from the LuaState stack.
            if let Some(table_obj) = unsafe { t.as_ref() } {
                Value::Number(table_obj.length() as f64)
            } else {
                Value::Number(0.0)
            }
        }
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
        (Value::String(a_ref), Value::String(b_ref)) => {
            // SAFETY: GC does not run during VM execution; GcString refs are valid
            // because the string values are on the LuaState stack.
            let a_str = unsafe { a_ref.as_ref() };
            // SAFETY: Same justification as above — GC is not running during execution.
            let b_str = unsafe { b_ref.as_ref() };
            match (a_str, b_str) {
                (Some(a), Some(b)) => Ok(a.data() < b.data()),
                _ => Ok(false),
            }
        }
        _ => {
            if let Some(metamethod) = find_common_metamethod(l, lhs, rhs, "__lt") {
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
        (Value::String(a_ref), Value::String(b_ref)) => {
            // SAFETY: GC does not run during VM execution; same justification as exec_lt.
            let a_str = unsafe { a_ref.as_ref() };
            // SAFETY: Same justification — GC is not running during execution.
            let b_str = unsafe { b_ref.as_ref() };
            match (a_str, b_str) {
                (Some(a), Some(b)) => Ok(a.data() <= b.data()),
                _ => Ok(false),
            }
        }
        _ => {
            if let Some(metamethod) = find_common_metamethod(l, lhs, rhs, "__le") {
                return call_metamethod_bool(l, gc, stack_limit, metamethod, lhs, rhs);
            }
            if let Some(metamethod) = find_common_metamethod(l, lhs, rhs, "__lt") {
                return call_metamethod_bool(l, gc, stack_limit, metamethod, rhs, lhs)
                    .map(|result| !result);
            }
            Err(RuntimeError::new(
                "attempt to compare non-comparable values",
            ))
        }
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
    let result = call_value(l, gc, metamethod, &[lhs.clone(), rhs.clone()], Some(1));
    l.top = saved_top;
    let results = result?;
    Ok(results.first().is_some_and(|value| !value.is_false()))
}

fn call_metamethod_value(
    l: &mut LuaState,
    gc: &mut GarbageCollector,
    stack_limit: usize,
    metamethod: Value,
    args: &[Value],
) -> Result<Value, RuntimeError> {
    let saved_top = l.top;
    l.top = l.top.max(stack_limit);
    let result = call_value(l, gc, metamethod, args, Some(1));
    l.top = saved_top;
    let results = result?;
    Ok(results.first().cloned().unwrap_or(Value::Nil))
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
    if let Some(metamethod) = find_common_metamethod(l, lhs, rhs, "__eq") {
        call_metamethod_bool(l, gc, stack_limit, metamethod, lhs, rhs)
    } else {
        Ok(false)
    }
}

fn find_metamethod(l: &LuaState, lhs: &Value, rhs: &Value, name: &str) -> Option<Value> {
    value_metatable(l, lhs)
        .and_then(|metatable| lookup_metamethod(metatable, name))
        .or_else(|| {
            value_metatable(l, rhs).and_then(|metatable| lookup_metamethod(metatable, name))
        })
}

fn find_common_metamethod(l: &LuaState, lhs: &Value, rhs: &Value, name: &str) -> Option<Value> {
    let lhs_method =
        value_metatable(l, lhs).and_then(|metatable| lookup_metamethod(metatable, name));
    let rhs_method =
        value_metatable(l, rhs).and_then(|metatable| lookup_metamethod(metatable, name));
    match (lhs_method, rhs_method) {
        (Some(lhs_method), Some(rhs_method)) if values_equal(&lhs_method, &rhs_method) => {
            Some(lhs_method)
        }
        _ => None,
    }
}

fn value_metatable(l: &LuaState, value: &Value) -> Option<GcRef<Table>> {
    match value {
        Value::Table(table_ref) => {
            // SAFETY: compared table values are reachable during VM execution.
            unsafe { table_ref.as_ref() }.and_then(|table| table.metatable())
        }
        Value::Userdata(userdata_ref) => {
            // SAFETY: compared userdata values are reachable during VM execution.
            unsafe { userdata_ref.as_ref() }.and_then(|userdata| userdata.metatable())
        }
        Value::Nil => l.nil_metatable,
        Value::Boolean(_) => l.boolean_metatable,
        Value::Number(_) => l.number_metatable,
        _ => None,
    }
}

fn lookup_metamethod(metatable: GcRef<Table>, name: &str) -> Option<Value> {
    // SAFETY: metatable is held by a live value being compared.
    let metatable = unsafe { metatable.as_ref() }?;
    for (key, value) in metatable.hash_entries() {
        if string_value_data(key).is_some_and(|key| key == name) && !value.is_nil() {
            return Some(value.clone());
        }
    }
    None
}

fn string_value_data(value: &Value) -> Option<&str> {
    let Value::String(string_ref) = value else {
        return None;
    };
    // SAFETY: string value is reachable during VM execution.
    unsafe { string_ref.as_ref() }.map(|string| string.data())
}

fn to_arith_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => Some(*n),
        Value::String(s) => {
            // SAFETY: string value is reachable during VM execution.
            unsafe { s.as_ref() }.and_then(|string| string.data().trim().parse::<f64>().ok())
        }
        _ => None,
    }
}

fn to_concat_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            // SAFETY: string value is reachable during VM execution.
            unsafe { s.as_ref() }.map(|string| string.data().to_string())
        }
        Value::Number(n) => {
            if n.fract() == 0.0 && n.is_finite() {
                Some(format!("{n:.0}"))
            } else {
                Some(n.to_string())
            }
        }
        _ => None,
    }
}

/// 值相等
fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    match (lhs, rhs) {
        (Value::Nil, Value::Nil) => true,
        (Value::Boolean(a), Value::Boolean(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => {
            // Compare string content, not pointer identity
            // SAFETY: GC does not run during VM execution; string refs are valid
            // because they are on the LuaState stack or in proto constants.
            let a_str = unsafe { a.as_ref() };
            // SAFETY: Same justification — GC is not running during execution.
            let b_str = unsafe { b.as_ref() };
            match (a_str, b_str) {
                (Some(a), Some(b)) => a.data() == b.data(),
                _ => a.as_ptr() == b.as_ptr(),
            }
        }
        // GC pointer types: compare pointer identity (same as C++)
        (Value::Table(a), Value::Table(b)) => a.as_ptr() == b.as_ptr(),
        (Value::Function(a), Value::Function(b)) => a.as_ptr() == b.as_ptr(),
        (Value::Userdata(a), Value::Userdata(b)) => a.as_ptr() == b.as_ptr(),
        (Value::Thread(a), Value::Thread(b)) => a.as_ptr() == b.as_ptr(),
        // Different types are never equal
        _ => false,
    }
}

/// 提取数值（含字符串强制转换）
fn as_number(val: &Value) -> f64 {
    match val {
        Value::Number(n) => *n,
        Value::Boolean(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::String(s) => {
            // Try to parse string as number
            // SAFETY: GC does not run during VM execution; same justification as above.
            if let Some(gc_str) = unsafe { s.as_ref() } {
                gc_str.data().trim().parse::<f64>().unwrap_or(0.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

/// 值转字符串（调试和 CONCAT 使用）
fn value_to_string(val: &Value) -> String {
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
        Value::String(s) => {
            // SAFETY: GC does not run during VM execution; GcString is valid
            // because the string value is on the LuaState stack or in constants.
            if let Some(gc_str) = unsafe { s.as_ref() } {
                gc_str.data().to_string()
            } else {
                String::new()
            }
        }
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
}

impl RuntimeError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            error_value: None,
        }
    }

    pub fn with_value(value: Value) -> Self {
        Self {
            message: value_to_string(&value),
            error_value: Some(value),
        }
    }

    pub fn error_value(&self) -> Option<Value> {
        self.error_value.clone()
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RuntimeError {}
