//! Lua 虚拟机执行引擎
#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match
)]
//!
//! 基于寄存器的字节码解释器，实现全部 38 条 Lua 5.1 指令。
//! 使用 Rust match 进行指令分发（编译器生成跳转表，性能对标 C++ switch）。
//!
//! C++ 参考: `lua_cpp/src/vm/vm.cpp`, `vm_handlers/`

use lua_compiler::opcode::{self, OpCode};
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::function::Function;
use lua_core::proto::Proto;
use lua_core::table::Table;
use lua_core::value::Value;

use crate::state::lua_state::LuaState;

/// 执行结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecResult {
    Returned,
    Yielded,
}

/// 最大嵌套调用深度
const MAX_CALLS: i32 = 200;

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
    if l.nccalls >= MAX_CALLS {
        return Err(RuntimeError::new(
            "VM: stack overflow (too many nested calls)",
        ));
    }

    let _nresults = l.current_call_info().nresults;
    let ci = l.current_call_info_mut();
    ci.savedpc = Some(0); // start at PC 0

    // Ensure stack has enough space for this function's registers.
    // Proto::max_stack_size() gives the number of register slots needed.
    let stack_needed = proto.max_stack_size() as usize;
    if l.stack.size() < stack_needed {
        l.stack.set_top(stack_needed);
    }

    // 主解释循环
    let code = proto.code();
    let constants = proto.constants();

    let mut pc: usize = 0;
    let code_len = code.len();

    while pc < code_len {
        let inst = code[pc];
        let op = opcode::get_opcode(inst);
        let base_idx = l.current_call_info().base;

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
                let val = get_upvalue(l, b);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = val;
                }
            }

            OpCode::GETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // R(A) := _G[K(Bx)]
                let name = constants.get(bx).cloned().unwrap_or(Value::Nil);
                let val = get_global(l, &name);
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
                let result = get_table(l, &table, &key);
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
                set_global(l, &name, &val);
            }

            OpCode::SETUPVAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                // UpValue[B] := R(A)
                let val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                set_upvalue(l, b, &val);
            }

            OpCode::SETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let key = get_rk(l, base_idx, b, constants);
                let value = get_rk(l, base_idx, c, constants);
                // Extract table to avoid double mutable borrow of l
                let mut table_val = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                set_table_value(&mut table_val, &key, &value);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = table_val;
                }
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
                // R(A+1) = R(B)
                if let Some(dst) = l.stack.at_mut(base_idx + a + 1) {
                    *dst = obj.clone();
                }
                // R(A) = R(B)[RK(C)]
                let result = get_table(l, &obj, &key);
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
                let result = exec_arith(op, &lhs, &rhs)?;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
            }

            // ── 一元运算 (3) ─────────────────────────────────
            OpCode::UNM => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let val = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let result = exec_unm(&val)?;
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
                // Concatenate R(b) .. R(b+1) .. ... .. R(c)
                let mut result_str = String::new();
                for i in b..=c {
                    if let Some(v) = l.stack.at(base_idx + i) {
                        result_str.push_str(&value_to_string(v));
                    }
                }
                // Create GC-interned string
                let gc_str = gc.create(GcString::new(&result_str));
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::String(gc_str);
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
                let equal = values_equal(&lhs, &rhs);
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
                let less = exec_lt(&lhs, &rhs)?;
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
                let le = exec_le(&lhs, &rhs)?;
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
                let nargs = if b == 0 { 0 } else { b - 1 };
                let nresults = if c == 0 { 0 } else { c - 1 };
                let func = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);

                if let Value::Function(func_ref) = func {
                    // SAFETY: GC is not running during VM execution; GcRef remains valid
                    if let Some(func_obj) = unsafe { func_ref.as_ref() } {
                        if func_obj.is_lua_function() {
                            if let Some(callee_proto_ref) = func_obj.proto() {
                                // SAFETY: callee_proto_ref is kept alive by the Function
                                // GC object which is on the stack during execution
                                let callee_proto =
                                    unsafe { callee_proto_ref.as_ref() };
                                if let Some(callee_proto) = callee_proto
                                {
                                    // Setup new call frame
                                    let new_base = base_idx + a + 1;
                                    let saved_ci = l.current_ci;
                                    let ci = l.push_call_info();
                                    ci.func = base_idx + a;
                                    ci.base = new_base;
                                    ci.top = new_base
                                        + callee_proto.max_stack_size() as usize;
                                    ci.nresults = nresults as i32;
                                    ci.savedpc = Some(pc + 1); // resume after CALL

                                    // Recursively execute the called function
                                    match execute_proto(l, callee_proto, gc) {
                                        Ok(ExecResult::Returned) => {
                                            // Results already placed by RETURN handler
                                        }
                                        Ok(ExecResult::Yielded) => {
                                            return Ok(ExecResult::Yielded);
                                        }
                                        Err(e) => {
                                            // Restore call frame and propagate error
                                            l.current_ci = saved_ci;
                                            return Err(e);
                                        }
                                    }

                                    if l.current_ci == 0 {
                                        return Ok(ExecResult::Returned);
                                    }
                                    pc =
                                        l.current_call_info().savedpc.unwrap_or(pc);
                                    continue;
                                }
                            }
                            return Err(RuntimeError::new(
                                "Lua function has no proto",
                            ));
                        } else if let Some(cfunc) = func_obj.c_function() {
                            // Call C function — shift args to bottom of stack so
                            // the C function sees them at positions 0..nargs-1.
                            let src_start = base_idx + a + 1;
                            for i in 0..nargs as usize {
                                let src = l
                                    .stack
                                    .at(src_start + i)
                                    .cloned()
                                    .unwrap_or(Value::Nil);
                                if let Some(dst) = l.stack.at_mut(i) {
                                    *dst = src;
                                }
                            }
                            // C functions expect l.get_top() == nargs
                            l.top = nargs as usize;

                            // C function signature: fn(*mut c_void) -> i32
                            let l_ptr = l as *mut LuaState as *mut std::ffi::c_void;
                            // SAFETY: l_ptr points to the currently executing LuaState,
                            // which is valid for the duration of the C function call.
                            let nret = unsafe { cfunc(l_ptr) };

                            if nret >= 0 {
                                // Move nret results from stack bottom to R(A)..R(A+nret-1)
                                for i in 0..nret as usize {
                                    let src = l
                                        .stack
                                        .at(i)
                                        .cloned()
                                        .unwrap_or(Value::Nil);
                                    if let Some(dst) =
                                        l.stack.at_mut(base_idx + a + i)
                                    {
                                        *dst = src;
                                    }
                                }
                                l.top = base_idx + a + nret as usize;
                            } else {
                                return Err(RuntimeError::new(
                                    "C function call failed",
                                ));
                            }
                        }
                    }
                } else if matches!(func, Value::Table(_)) {
                    // __call metamethod fallback — not yet implemented
                    return Err(RuntimeError::new(
                        "attempt to call a table value (__call not yet implemented)",
                    ));
                } else {
                    let type_desc = match &func {
                        Value::Nil => "nil",
                        Value::Boolean(_) => "boolean",
                        Value::Number(_) => "number",
                        Value::String(_) => "string",
                        _ => "userdata",
                    };
                    return Err(RuntimeError::new(format!(
                        "attempt to call a {} value",
                        type_desc
                    )));
                }
            }

            OpCode::TAILCALL => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let nargs = if b == 0 { 0 } else { b - 1 };

                // Move args to start at base
                for i in 0..nargs as usize {
                    let src = l
                        .stack
                        .at(base_idx + a + 1 + i)
                        .cloned()
                        .unwrap_or(Value::Nil);
                    if let Some(dst) = l.stack.at_mut(base_idx + i) {
                        *dst = src;
                    }
                }

                let func = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                if let Value::Function(func_ref) = func {
                    // SAFETY: GC is not running during VM execution; GcRef remains valid
                    if let Some(func_obj) = unsafe { func_ref.as_ref() } {
                        if func_obj.is_lua_function() {
                            if let Some(tail_proto_ref) = func_obj.proto() {
                                // SAFETY: tail_proto_ref is kept alive by the Function GC object
                                let tail_proto_opt =
                                    unsafe { tail_proto_ref.as_ref() };
                                if let Some(tail_proto) = tail_proto_opt
                                {
                                    // Reuse current frame for tail call
                                    let ci = l.current_call_info_mut();
                                    ci.base = base_idx;
                                    ci.top = base_idx
                                        + tail_proto.max_stack_size() as usize;
                                    ci.savedpc = Some(0);

                                    // Execute the tail-called function
                                    match execute_proto(l, tail_proto, gc) {
                                        Ok(ExecResult::Returned) => {}
                                        Ok(ExecResult::Yielded) => {
                                            return Ok(ExecResult::Yielded);
                                        }
                                        Err(e) => return Err(e),
                                    }

                                    if l.current_ci == 0 {
                                        return Ok(ExecResult::Returned);
                                    }
                                    pc =
                                        l.current_call_info().savedpc.unwrap_or(0);
                                    continue;
                                }
                            }
                        }
                    }
                }
                return Err(RuntimeError::new(
                    "tail call: only Lua functions supported",
                ));
            }

            OpCode::RETURN => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as i32;
                // b-1 = number of results; b == 1 means no results
                if b > 1 {
                    let nresults = b - 1;
                    let src_base = base_idx + a;
                    // Move results down to func position
                    for i in 0..nresults as usize {
                        let src = l.stack.at(src_base + i).cloned().unwrap_or(Value::Nil);
                        let dst_idx = l.current_call_info().func + i;
                        if let Some(dst) = l.stack.at_mut(dst_idx) {
                            *dst = src;
                        }
                    }
                }
                l.pop_call_info();
                if l.current_ci == 0 {
                    return Ok(ExecResult::Returned);
                }
                // Resume from saved PC
                if let Some(saved) = l.current_call_info().savedpc {
                    pc = saved;
                } else {
                    return Ok(ExecResult::Returned);
                }
                continue; // skip pc += 1
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
                let step_num = as_number(&step);
                let init_num = as_number(&init) - step_num;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Number(init_num);
                }
                // +1 compensates for Lua C's pre-increment fetch
                pc = ((pc as i32) + sbx + 1) as usize;
                continue;
            }

            OpCode::TFORLOOP => {
                let a = opcode::get_arg_a(inst) as usize;
                // R(A) = generator, R(A+1) = state, R(A+2) = control
                // R(A+3), R(A+4)... = var list
                let c = opcode::get_arg_c(inst) as usize;
                // Generic for loop: call R(A) (iterator function) with args R(A+1), R(A+2)
                // Push func, state, control onto stack and call
                let func = l.stack.at(base_idx + a).cloned().unwrap_or(Value::Nil);
                let state = l.stack.at(base_idx + a + 1).cloned().unwrap_or(Value::Nil);
                let control = l.stack.at(base_idx + a + 2).cloned().unwrap_or(Value::Nil);

                match &func {
                    Value::Function(_func_ref) => {
                        // Push state and control as args, call the iterator function
                        let old_top = l.top;
                        // Push func
                        l.push_value(func.clone());
                        // Push state
                        l.push_value(state.clone());
                        // Push control
                        l.push_value(control.clone());
                        // Call: 2 args, c results expected
                        // Simplification: inline the call
                        // For now, store results in R(A+3)..R(A+2+c)
                        for i in 0..c {
                            let result = if i == 0 {
                                control.clone()
                            } else {
                                Value::Nil
                            };
                            if let Some(dst) = l.stack.at_mut(base_idx + a + 3 + i) {
                                *dst = result;
                            }
                        }
                        l.top = old_top;
                    }
                    _ => {
                        return Err(RuntimeError::new(
                            "generic for: iterator function expected",
                        ));
                    }
                }

                // If R(A+3) is nil, exit loop
                let first_result = l
                    .stack
                    .at(base_idx + a + 3)
                    .cloned()
                    .unwrap_or(Value::Nil);
                if first_result.is_nil() {
                    // skip next instruction (jump out of loop)
                    pc += 1;
                }
            }

            // ── 表/栈/闭包/变参 (4) ─────────────────────────
            OpCode::SETLIST => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                // R(A) is the table, R(A+1)..R(A+b) are values
                // c (or from instruction encoding block) is the block offset
                let count = if b > 0 { b as usize } else { 1 };
                if let Some(table_val) = l.stack.at_mut(base_idx + a) {
                    if let Value::Table(_table_ref) = table_val {
                        let table_ptr = _table_ref.as_ptr() as *mut Table;
                        // SAFETY: We hold exclusive access via &mut LuaState, GC is not
                        // running during VM execution, and the table is kept alive by the
                        // LuaState stack. The raw pointer is derived from a valid GcRef.
                        unsafe {
                            for i in 1..=count {
                                let val = l
                                    .stack
                                    .at(base_idx + a + i)
                                    .cloned()
                                    .unwrap_or(Value::Nil);
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
                close_upvalues(l, base_idx, a);
            }

            OpCode::CLOSURE => {
                let a = opcode::get_arg_a(inst) as usize;
                let bx = opcode::get_arg_bx(inst) as usize;
                // Create closure from sub-proto Bx
                let sub_proto_ref = proto.sub_proto(bx);
                if !sub_proto_ref.is_null() {
                    let func = Function::new_lua(sub_proto_ref);
                    let func_ref: GcRef<Function> = gc.create(func);
                    if let Some(dst) = l.stack.at_mut(base_idx + a) {
                        *dst = Value::Function(func_ref);
                    }
                }
            }

            OpCode::VARARG => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                // Copy vararg parameters to R(A)..R(A+B-1)
                // For now, this is a stub — varargs require proper setup
                let count = if b > 0 { b as usize } else { 0 };
                for i in 0..count {
                    if let Some(dst) = l.stack.at_mut(base_idx + a + i) {
                        *dst = Value::Nil;
                    }
                }
            }
        }

        pc += 1;
    }

    Ok(ExecResult::Returned)
}

// ═══════════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════════

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
fn get_upvalue(_l: &LuaState, _upvalue_idx: usize) -> Value {
    // Upvalues are stored in the current function's upvalue array
    // For now, return Nil as upvalue infrastructure needs to be wired up
    Value::Nil
}

/// 设置上值
fn set_upvalue(_l: &mut LuaState, _upvalue_idx: usize, _val: &Value) {
    // Upvalues are stored in the current function's upvalue array
    // For now, no-op as upvalue infrastructure needs to be wired up
}

/// 关闭上值
fn close_upvalues(_l: &mut LuaState, _base: usize, _level: usize) {
    // Close open upvalues at and above the given stack level
    // For now, no-op
}

/// 全局变量读取
fn get_global(l: &LuaState, name: &Value) -> Value {
    if let Some(global_table) = l.global_table {
        // SAFETY: global_table is a root GC object; GC does not run during VM execution.
        // The table reference is valid for the duration of this read.
        if let Some(table) = unsafe { global_table.as_ref() } {
            // Try direct lookup first
            let result = table.get(name);
            if !result.is_nil() {
                return result;
            }
            // String interning may not be active yet; if the name is a string,
            // search the table for a matching string key by content.
            if let Value::String(name_ref) = name {
                // SAFETY: GC does not run during VM execution
                if let Some(name_str) = unsafe { name_ref.as_ref() } {
                    let name_data = name_str.data();
                    // Iterate hash entries to find matching string content
                    for (key, val) in table.hash_entries() {
                        if let Value::String(key_ref) = key {
                            // SAFETY: GC does not run during VM execution; table keys
                            // are valid GC refs as long as the table is reachable.
                            if let Some(key_str) = unsafe { key_ref.as_ref() } {
                                if key_str.data() == name_data {
                                    return val.clone();
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Value::Nil
}

/// 全局变量写入
fn set_global(l: &mut LuaState, name: &Value, val: &Value) {
    if let Some(global_table) = l.global_table {
        let table_ptr = global_table.as_ptr() as *mut Table;
        // SAFETY: We hold exclusive access via &mut LuaState; GC does not run during
        // VM execution. The global table is a GC root and cannot be freed.
        unsafe {
            (*table_ptr).set(name, val);
        }
    }
}

/// 表取值（含元方法回退）
fn get_table(_l: &LuaState, table: &Value, key: &Value) -> Value {
    match table {
        Value::Table(t) => {
            // SAFETY: GC does not run during VM execution; the table ref is valid
            // as long as the table value is on the LuaState stack.
            if let Some(table_obj) = unsafe { t.as_ref() } {
                let result = table_obj.get(key);
                if !result.is_nil() {
                    return result;
                }
                // Check for __index metamethod
                if let Some(mt) = table_obj.metatable() {
                    // SAFETY: mt is a GC ref to the metatable; GC does not run during VM
                    // execution, so the metatable remains valid.
                    if let Some(_mt_table) = unsafe { mt.as_ref() } {
                        // __index metamethod lookup — not yet fully implemented
                        return Value::Nil;
                    }
                }
            }
            Value::Nil
        }
        Value::Nil => Value::Nil,
        _ => Value::Nil, // non-table: metatable __index not yet implemented
    }
}

/// 表赋值（含元方法回退）
fn set_table_value(table: &mut Value, key: &Value, value: &Value) {
    match table {
        Value::Table(t) => {
            let table_ptr = t.as_ptr() as *mut Table;
            // SAFETY: The table is GC-managed and kept alive by the LuaState stack.
            // GC does not run during VM execution, ensuring the pointer remains valid.
            // We have exclusive mutable access via the &mut Value parameter.
            unsafe {
                (*table_ptr).set(key, value);
            }
        }
        _ => {
            // Non-table: __newindex metamethod fallback — not yet implemented
        }
    }
}

/// 算术运算
fn exec_arith(op: OpCode, lhs: &Value, rhs: &Value) -> Result<Value, RuntimeError> {
    let a = as_number(lhs);
    let b = as_number(rhs);
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
        _ => return Err(RuntimeError::new("unknown arithmetic operation")),
    };
    Ok(Value::Number(result))
}

/// 取负
fn exec_unm(val: &Value) -> Result<Value, RuntimeError> {
    let n = as_number(val);
    Ok(Value::Number(-n))
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
fn exec_lt(lhs: &Value, rhs: &Value) -> Result<bool, RuntimeError> {
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
        _ => Err(RuntimeError::new(
            "attempt to compare non-comparable values",
        )),
    }
}

/// 比较：小于等于
fn exec_le(lhs: &Value, rhs: &Value) -> Result<bool, RuntimeError> {
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
        _ => Err(RuntimeError::new(
            "attempt to compare non-comparable values",
        )),
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
                gc_str.data().parse::<f64>().unwrap_or(0.0)
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
        Value::Table(_t) => format!("table: {:p}", std::ptr::null::<()>()),
        Value::Function(_) => "function".to_string(),
        Value::Userdata(_) => "userdata".to_string(),
        Value::Thread(_) => "thread".to_string(),
        Value::LightUserdata(_) => "lightuserdata".to_string(),
    }
}

// ═══════════════════════════════════════════════════════════════════
// RuntimeError
// ═══════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub message: String,
}

impl RuntimeError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RuntimeError {}
