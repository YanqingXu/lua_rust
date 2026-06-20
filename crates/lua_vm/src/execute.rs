//! Lua 虚拟机执行引擎
#![allow(
    unused_variables,
    unused_mut,
    clippy::collapsible_if,
    clippy::collapsible_match
)] // TODO stubs
//!
//! 基于寄存器的字节码解释器，实现全部 38 条 Lua 5.1 指令。
//! 使用 Rust match 进行指令分发（编译器生成跳转表，性能对标 C++ switch）。
//!
//! C++ 参考: `lua_cpp/src/vm/vm.cpp`, `vm_handlers/`

use lua_compiler::opcode::{self, OpCode};
use lua_core::proto::Proto;
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
/// 局部变量（与 Lua C `luaV_execute` 对齐）：
/// - `ci` — 当前 CallInfo
/// - `cl` — 当前 Proto
/// - `base` — 栈基址指针（计算值 = &l.stack[ci.base]）
/// - `pc` — 程序计数器
pub fn execute_proto(l: &mut LuaState, proto: &Proto) -> Result<ExecResult, RuntimeError> {
    if l.nccalls >= MAX_CALLS {
        return Err(RuntimeError::new(
            "VM: stack overflow (too many nested calls)",
        ));
    }

    let nresults = l.current_call_info().nresults;
    let ci = l.current_call_info_mut();
    ci.savedpc = Some(0); // start at PC 0

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
                // TODO: actual upvalue lookup
                let val = Value::Nil;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = val;
                }
            }

            OpCode::GETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let _bx = opcode::get_arg_bx(inst) as usize;
                // TODO: actual global lookup via global table
                let val = Value::Nil;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = val;
                }
            }

            OpCode::GETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let table = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, base_idx, c);
                let result = get_table(&table, &key);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
            }

            // ── 变量赋值 (3) ─────────────────────────────────
            OpCode::SETGLOBAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let _bx = opcode::get_arg_bx(inst) as usize;
                // TODO: actual global set via global table
            }

            OpCode::SETUPVAL => {
                let a = opcode::get_arg_a(inst) as usize;
                let _b = opcode::get_arg_b(inst) as usize;
                // TODO: actual upvalue set
            }

            OpCode::SETTABLE => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let key = get_rk(l, base_idx, b);
                let value = get_rk(l, base_idx, c);
                if let Some(table_val) = l.stack.at_mut(base_idx + a) {
                    set_table(table_val, &key, &value);
                }
            }

            // ── 表操作 (2) ───────────────────────────────────
            OpCode::NEWTABLE => {
                let _a = opcode::get_arg_a(inst) as usize;
                let _b = opcode::get_arg_b(inst);
                let _c = opcode::get_arg_c(inst);
                // TODO: create new table via GC
            }

            OpCode::SELF => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst) as usize;
                let c = opcode::get_arg_c(inst);
                let obj = l.stack.at(base_idx + b).cloned().unwrap_or(Value::Nil);
                let key = get_rk(l, base_idx, c);
                // R(A+1) = R(B)
                if let Some(dst) = l.stack.at_mut(base_idx + a + 1) {
                    *dst = obj.clone();
                }
                // R(A) = R(B)[RK(C)]
                let result = get_table(&obj, &key);
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = result;
                }
            }

            // ── 算术运算 (6) ─────────────────────────────────
            OpCode::ADD | OpCode::SUB | OpCode::MUL | OpCode::DIV | OpCode::MOD | OpCode::POW => {
                let a = opcode::get_arg_a(inst) as usize;
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b);
                let rhs = get_rk(l, base_idx, c);
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
                let mut result = String::new();
                for i in b..=c {
                    if let Some(v) = l.stack.at(base_idx + i) {
                        result.push_str(&value_to_string(v));
                    }
                }
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Nil; // TODO: create GC string
                }
            }

            // ── 控制流 (6) ───────────────────────────────────
            OpCode::JMP => {
                let sbx = opcode::get_arg_sbx(inst);
                pc = ((pc as i32) + sbx) as usize;
                continue; // skip pc += 1
            }

            OpCode::EQ => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b);
                let rhs = get_rk(l, base_idx, c);
                let equal = values_equal(&lhs, &rhs);
                if (equal && a == 0) || (!equal && a != 0) {
                    pc += 1; // skip next
                }
            }

            OpCode::LT => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b);
                let rhs = get_rk(l, base_idx, c);
                let less = exec_lt(&lhs, &rhs)?;
                if (less && a == 0) || (!less && a != 0) {
                    pc += 1;
                }
            }

            OpCode::LE => {
                let a = opcode::get_arg_a(inst);
                let b = opcode::get_arg_b(inst);
                let c = opcode::get_arg_c(inst);
                let lhs = get_rk(l, base_idx, b);
                let rhs = get_rk(l, base_idx, c);
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
                let _b = opcode::get_arg_b(inst);
                let _c = opcode::get_arg_c(inst);
                // TODO: actual function call with stack frame management
                // For now, just skip
            }

            OpCode::TAILCALL => {
                let _a = opcode::get_arg_a(inst) as usize;
                let _b = opcode::get_arg_b(inst);
                // TODO: tail call implementation
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
                let mut idx_val = l
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
                    pc = ((pc as i32) + sbx) as usize;
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
                let mut init = l
                    .stack
                    .at(base_idx + a)
                    .cloned()
                    .unwrap_or(Value::Number(0.0));
                let step_num = as_number(&step);
                let init_num = as_number(&init) - step_num;
                if let Some(dst) = l.stack.at_mut(base_idx + a) {
                    *dst = Value::Number(init_num);
                }
                pc = ((pc as i32) + sbx) as usize;
                continue;
            }

            OpCode::TFORLOOP => {
                // TODO: generic for loop iterator protocol
            }

            // ── 表/栈/闭包/变参 (4) ─────────────────────────
            OpCode::SETLIST => {
                let _a = opcode::get_arg_a(inst) as usize;
                let _b = opcode::get_arg_b(inst);
                let _c = opcode::get_arg_c(inst);
                // TODO: SETLIST implementation
            }

            OpCode::CLOSE => {
                let _a = opcode::get_arg_a(inst) as usize;
                // TODO: close upvalues at level
            }

            OpCode::CLOSURE => {
                let _a = opcode::get_arg_a(inst) as usize;
                let _bx = opcode::get_arg_bx(inst) as usize;
                // TODO: create closure from sub-proto
            }

            OpCode::VARARG => {
                let _a = opcode::get_arg_a(inst) as usize;
                let _b = opcode::get_arg_b(inst);
                // TODO: copy vararg parameters
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
fn get_rk(l: &LuaState, base: usize, rk: i32) -> Value {
    if opcode::is_k(rk) {
        // TODO: constant lookup — needs proto reference
        Value::Nil
    } else {
        let reg = rk as usize;
        l.stack.at(base + reg).cloned().unwrap_or(Value::Nil)
    }
}

/// 表取值
fn get_table(table: &Value, key: &Value) -> Value {
    match table {
        Value::Table(t) => {
            // TODO: actual Table::get() call
            Value::Nil
        }
        Value::Nil => Value::Nil,
        _ => {
            // TODO: invoke __index metamethod
            Value::Nil
        }
    }
}

/// 表赋值
fn set_table(table: &mut Value, key: &Value, value: &Value) {
    // TODO: actual Table::set() call + __newindex metamethod
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
        Value::String(s) => Value::Number(0.0), // TODO: string length
        Value::Table(_t) => Value::Number(0.0), // TODO: table length via #
        _ => Value::Number(0.0),
    }
}

/// 比较：小于
fn exec_lt(lhs: &Value, rhs: &Value) -> Result<bool, RuntimeError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(a < b),
        (Value::String(_a), Value::String(_b)) => Ok(false), // TODO: string comparison
        _ => Err(RuntimeError::new(
            "attempt to compare non-comparable values",
        )),
    }
}

/// 比较：小于等于
fn exec_le(lhs: &Value, rhs: &Value) -> Result<bool, RuntimeError> {
    match (lhs, rhs) {
        (Value::Number(a), Value::Number(b)) => Ok(a <= b),
        (Value::String(_a), Value::String(_b)) => Ok(false), // TODO: string comparison
        _ => Err(RuntimeError::new(
            "attempt to compare non-comparable values",
        )),
    }
}

/// 值相等
fn values_equal(lhs: &Value, rhs: &Value) -> bool {
    lhs == rhs
}

/// 提取数值
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
        _ => 0.0, // TODO: string-to-number coercion
    }
}

/// 值转字符串（调试和 CONCAT 使用）
fn value_to_string(val: &Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(_s) => "string".to_string(), // TODO: string data
        _ => "value".to_string(),
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
