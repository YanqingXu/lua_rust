//! 表达式发射器 (Expression Emitter)
//!
//! 实现 AST 表达式 → 字节码的 lowering：ValueResult（右值）、
//! CondResult（条件）、LValueRef（左值）、CallResultInfo（调用）和全部 14 种 Expr 节点。
//!

use crate::ast::expr::{
    BinaryExpr, BinaryOp, CallExpr, Expr, FunctionExpr, IndexExpr, MemberExpr, TableExpr,
    UnaryExpr, UnaryOp,
};
use crate::codegen::CodeGenerator;
use crate::codegen::types::{
    AccessKind, CallResultInfo, CallResultKind, CondResult, ImmediateKind, LValueKind, LValueRef,
    NO_JUMP, PatchList, ValueResult,
};
use crate::opcode::{self, LFIELDS_PER_FLUSH, MAXARG_C, MAXINDEXRK, OpCode};

use crate::opcode::rk_ask;

impl CodeGenerator {
    // ═══════════════════════════════════════════════════════════════
    // 公开入口
    // ═══════════════════════════════════════════════════════════════

    /// 发射表达式右值
    ///
    pub fn emit_value(&mut self, expr: &Expr) -> ValueResult {
        match expr {
            Expr::Nil(_) => ValueResult::make_nil(),
            Expr::Boolean(b) => ValueResult::make_boolean(b.value),
            Expr::Number(n) => ValueResult::make_number(n.value),
            Expr::String(s) => {
                // SAFETY: self.gc is a valid pointer set during CodeGenerator::new()
                let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
                let k = self
                    .builder
                    .add_string_constant(gc, &s.value)
                    .unwrap_or_else(|| {
                        // Fallback: add as number constant if string constant fails
                        self.builder.add_number_constant(0.0)
                    });
                ValueResult::make_constant(k)
            }
            Expr::Vararg(_) => {
                let info = self.emit_vararg_expr();
                ValueResult::make_multi_ret(AccessKind::Vararg, info.base_reg, info.instruction_pc)
            }
            Expr::Name(n) => {
                let sym = self.resolve_name(&n.name);
                self.symbol_to_value(&sym)
            }
            Expr::Binary(b) => self.emit_value_binary(b),
            Expr::Unary(u) => self.emit_value_unary(u),
            Expr::Table(t) => self.emit_value_table(t),
            Expr::Call(c) => {
                let info = self.emit_call_expr(c, -1);
                ValueResult::make_multi_ret(AccessKind::Call, info.base_reg, info.instruction_pc)
            }
            Expr::Index(i) => self.emit_value_index(i),
            Expr::Member(m) => self.emit_value_member(m),
            Expr::Function(f) => self.emit_value_function_expr(f),
            Expr::Paren(p) => {
                let mut inner = self.emit_value(&p.expression);
                inner = self.force_single_value(inner);
                let reg = self.value_to_any_reg(inner);
                ValueResult::make_register(reg, true, AccessKind::None)
            }
        }
    }

    /// 发射条件结果
    ///
    pub fn emit_cond_result(&mut self, expr: &Expr) -> CondResult {
        // 短路求值: and/or
        if let Expr::Binary(b) = expr {
            match b.op {
                BinaryOp::And => {
                    let left = self.emit_cond_result(&b.left);
                    let right = self.emit_cond_result(&b.right);
                    return CondResult {
                        false_list: PatchList::merge(left.false_list, &right.false_list),
                        ..Default::default()
                    };
                }
                BinaryOp::Or => {
                    let left = self.emit_cond_result_true(&b.left);
                    let right = self.emit_cond_result(&b.right);
                    self.patch_list_to_here(&left.true_list);
                    return CondResult {
                        false_list: right.false_list,
                        ..Default::default()
                    };
                }
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let false_list = self.emit_comparison_jump(b, false);
                    return CondResult {
                        false_list,
                        ..Default::default()
                    };
                }
                _ => {}
            }
        }

        // not 条件反转
        if let Expr::Unary(u) = expr
            && u.op == UnaryOp::Not
        {
            let inner = self.emit_cond_result_true(&u.operand);
            return CondResult {
                false_list: inner.true_list,
                ..Default::default()
            };
        }

        // Native ValueResult fallback
        let mut val = self.emit_value(expr);
        val = self.force_single_value(val);
        let mut result = CondResult::default();

        match self.constant_truthiness(&val) {
            PayloadTruthiness::Falsy => {
                result.false_list.append(self.emit_jump());
            }
            PayloadTruthiness::Truthy => {
                // 无条件通过
            }
            PayloadTruthiness::Runtime => {
                let reg = self.value_to_any_reg(val);
                self.code_abc(OpCode::TEST, reg, 0, 0, self.current_line);
                result.false_list.append(self.emit_jump());
                self.free_reg(reg);
            }
        }
        result
    }

    /// 条件"真路径"结果（用于 and/or 短路求值的右侧）
    fn emit_cond_result_true(&mut self, expr: &Expr) -> CondResult {
        if let Expr::Binary(b) = expr {
            match b.op {
                BinaryOp::And => {
                    let left = self.emit_cond_result(&b.left);
                    let right = self.emit_cond_result_true(&b.right);
                    self.patch_list_to_here(&left.false_list);
                    return CondResult {
                        true_list: right.true_list,
                        ..Default::default()
                    };
                }
                BinaryOp::Or => {
                    let left = self.emit_cond_result_true(&b.left);
                    let right = self.emit_cond_result_true(&b.right);
                    return CondResult {
                        true_list: PatchList::merge(left.true_list, &right.true_list),
                        ..Default::default()
                    };
                }
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let true_list = self.emit_comparison_jump(b, true);
                    return CondResult {
                        true_list,
                        ..Default::default()
                    };
                }
                _ => {}
            }
        }

        if let Expr::Unary(u) = expr
            && u.op == UnaryOp::Not
        {
            let inner = self.emit_cond_result(&u.operand);
            return CondResult {
                true_list: inner.false_list,
                ..Default::default()
            };
        }

        let mut val = self.emit_value(expr);
        val = self.force_single_value(val);
        let mut result = CondResult::default();

        match self.constant_truthiness(&val) {
            PayloadTruthiness::Truthy => {
                result.true_list.append(self.emit_jump());
            }
            PayloadTruthiness::Falsy => {}
            PayloadTruthiness::Runtime => {
                let reg = self.value_to_any_reg(val);
                self.code_abc(OpCode::TEST, reg, 0, 1, self.current_line);
                result.true_list.append(self.emit_jump());
                self.free_reg(reg);
            }
        }
        result
    }

    // ═══════════════════════════════════════════════════════════════
    // 值物化 / 转换
    // ═══════════════════════════════════════════════════════════════

    /// 物化值到目标寄存器
    pub fn materialize_value(&mut self, val: ValueResult, reg: i32) {
        match val {
            ValueResult::None => {}
            ValueResult::Immediate {
                kind,
                boolean_value,
                number_value,
            } => match kind {
                ImmediateKind::Nil => {
                    self.code_abc(OpCode::LOADNIL, reg, reg, 0, self.current_line);
                }
                ImmediateKind::Boolean => {
                    self.code_abc(
                        OpCode::LOADBOOL,
                        reg,
                        if boolean_value { 1 } else { 0 },
                        0,
                        self.current_line,
                    );
                }
                ImmediateKind::Number => {
                    let k = self.builder.add_number_constant(number_value);
                    self.code_abx(OpCode::LOADK, reg, k, self.current_line);
                }
                ImmediateKind::None => {}
            },
            ValueResult::ConstantRef { const_index } => {
                self.code_abx(OpCode::LOADK, reg, const_index, self.current_line);
            }
            ValueResult::RegisterRef { reg: src_reg, .. } => {
                if src_reg != reg {
                    self.code_abc(OpCode::MOVE, reg, src_reg, 0, self.current_line);
                }
            }
            ValueResult::PendingLoad {
                access,
                reg: table_reg,
                const_index,
                aux,
            } => match access {
                AccessKind::Global => {
                    self.code_abx(OpCode::GETGLOBAL, reg, const_index, self.current_line);
                }
                AccessKind::Upvalue => {
                    self.code_abc(OpCode::GETUPVAL, reg, aux, 0, self.current_line);
                }
                AccessKind::Indexed => {
                    self.code_abc(OpCode::GETTABLE, reg, table_reg, aux, self.current_line);
                }
                _ => {}
            },
            ValueResult::Relocatable { instruction_pc } => {
                let mut inst = self.builder.instruction(instruction_pc).unwrap();
                opcode::set_arg_a(&mut inst, reg);
                self.builder.replace_instruction(instruction_pc, inst);
            }
            ValueResult::MultiRet {
                access,
                reg: _base_reg,
                instruction_pc,
            } => match access {
                AccessKind::Call => {
                    let mut inst = self.builder.instruction(instruction_pc).unwrap();
                    let call_base = opcode::get_arg_a(inst);
                    opcode::set_arg_c(&mut inst, 2); // C=2 → 1 return value
                    self.builder.replace_instruction(instruction_pc, inst);
                    if call_base != reg {
                        self.code_abc(OpCode::MOVE, reg, call_base, 0, self.current_line);
                    }
                }
                AccessKind::Vararg => {
                    let mut inst = self.builder.instruction(instruction_pc).unwrap();
                    opcode::set_arg_a(&mut inst, reg);
                    opcode::set_arg_b(&mut inst, 2); // B=2 → 1 value
                    self.builder.replace_instruction(instruction_pc, inst);
                }
                _ => {}
            },
            ValueResult::PendingJump { instruction_pc } => {
                let true_jump = instruction_pc;
                self.code_abc(OpCode::LOADBOOL, reg, 0, 1, self.current_line);
                let true_label = self.get_label();
                self.fix_jump(true_jump, true_label);
                self.code_abc(OpCode::LOADBOOL, reg, 1, 0, self.current_line);
            }
        }
    }

    /// 值 → RK 操作数
    pub fn value_to_rk(&mut self, val: ValueResult) -> i32 {
        match &val {
            ValueResult::Immediate {
                kind, number_value, ..
            } => {
                if *kind == ImmediateKind::Number {
                    let k = self.builder.add_number_constant(*number_value);
                    if k <= MAXINDEXRK {
                        return rk_ask(k);
                    }
                }
            }
            ValueResult::ConstantRef { const_index } if *const_index <= MAXINDEXRK => {
                return rk_ask(*const_index);
            }
            _ => {}
        }
        self.value_to_any_reg(val)
    }

    /// 值落到任意寄存器
    pub fn value_to_any_reg(&mut self, val: ValueResult) -> i32 {
        match &val {
            ValueResult::RegisterRef { reg, .. } => *reg,
            ValueResult::MultiRet {
                access: AccessKind::Call,
                instruction_pc,
                ..
            } => {
                let mut inst = self.builder.instruction(*instruction_pc).unwrap();
                opcode::set_arg_c(&mut inst, 2);
                self.builder.replace_instruction(*instruction_pc, inst);
                opcode::get_arg_a(inst)
            }
            _ => {
                let reg = self.reg_alloc.alloc();
                self.materialize_value(val, reg);
                reg
            }
        }
    }

    /// 强制为单值（将 MultiRet 收敛）
    pub fn force_single_value(&mut self, val: ValueResult) -> ValueResult {
        match &val {
            ValueResult::MultiRet {
                access: AccessKind::Vararg,
                instruction_pc,
                ..
            } => {
                let mut inst = self.builder.instruction(*instruction_pc).unwrap();
                opcode::set_arg_b(&mut inst, 2);
                self.builder.replace_instruction(*instruction_pc, inst);
                ValueResult::make_relocatable(*instruction_pc)
            }
            ValueResult::MultiRet {
                access: AccessKind::Call,
                instruction_pc,
                ..
            } => {
                let mut inst = self.builder.instruction(*instruction_pc).unwrap();
                opcode::set_arg_c(&mut inst, 2);
                self.builder.replace_instruction(*instruction_pc, inst);
                ValueResult::make_register(opcode::get_arg_a(inst), false, AccessKind::None)
            }
            _ => val,
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // 二元表达式 lowering
    // ═══════════════════════════════════════════════════════════════

    fn emit_value_binary(&mut self, e: &BinaryExpr) -> ValueResult {
        let op = e.op;

        // 比较 → 条件通道 + 物化
        if matches!(
            op,
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge
        ) {
            let cond = CondResult {
                true_list: self.emit_comparison_jump(e, true),
                ..Default::default()
            };
            let result_reg = self.reg_alloc.alloc();
            self.materialize_cond_result(&cond, result_reg, false);
            return ValueResult::make_register(result_reg, true, AccessKind::None);
        }

        // And/Or 短路求值
        if op == BinaryOp::And || op == BinaryOp::Or {
            let left = self.emit_value(&e.left);
            let left = self.force_single_value(left);
            let result_reg = self.reg_alloc.alloc();
            self.materialize_value(left, result_reg);
            let test_cond = if op == BinaryOp::And { 0 } else { 1 };
            self.code_abc(OpCode::TEST, result_reg, 0, test_cond, self.current_line);
            let skip_right = self.code_as_bx(OpCode::JMP, 0, NO_JUMP, self.current_line);
            let right = self.emit_value(&e.right);
            let right = self.force_single_value(right);
            self.materialize_value(right, result_reg);
            self.fix_jump(skip_right, self.get_label());
            return ValueResult::make_register(result_reg, true, AccessKind::None);
        }

        // Concat
        if op == BinaryOp::Concat {
            let left = self.emit_value(&e.left);
            let left = self.force_single_value(left);
            let left_reg = self.value_to_any_reg(left);
            let right = self.emit_value(&e.right);
            let right = self.force_single_value(right);
            let right_reg = self.value_to_any_reg(right);
            let target = self.reg_alloc.current();
            self.reg_alloc.reserve(2);
            let _ = self
                .reg_alloc
                .check_stack(0, self.builder.max_stack_size() as i32);
            self.materialize_value(
                ValueResult::make_register(left_reg, false, AccessKind::None),
                target,
            );
            self.materialize_value(
                ValueResult::make_register(right_reg, false, AccessKind::None),
                target + 1,
            );
            self.code_abc(
                OpCode::CONCAT,
                target,
                target,
                target + 1,
                self.current_line,
            );
            self.reg_alloc.set_freereg(target + 1);
            return ValueResult::make_register(target, true, AccessKind::None);
        }

        // 算术运算
        let arith_op = match op {
            BinaryOp::Add => OpCode::ADD,
            BinaryOp::Sub => OpCode::SUB,
            BinaryOp::Mul => OpCode::MUL,
            BinaryOp::Div => OpCode::DIV,
            BinaryOp::Mod => OpCode::MOD,
            BinaryOp::Pow => OpCode::POW,
            _ => OpCode::ADD,
        };

        let left = self.emit_value(&e.left);
        let right = self.emit_value(&e.right);

        // 尝试常量折叠
        if let (
            ValueResult::Immediate {
                kind: ImmediateKind::Number,
                number_value: ln,
                ..
            },
            ValueResult::Immediate {
                kind: ImmediateKind::Number,
                number_value: rn,
                ..
            },
        ) = (&left, &right)
            && let Some(folded) = Self::fold_arithmetic(op, *ln, *rn)
        {
            return ValueResult::make_number(folded);
        }

        let rk_left = self.value_to_rk(left);
        let rk_right = self.value_to_rk(right);

        if rk_left > rk_right {
            self.free_reg(rk_left);
            self.free_reg(rk_right);
        } else {
            self.free_reg(rk_right);
            self.free_reg(rk_left);
        }

        let result_reg = self.reg_alloc.alloc();
        self.code_abc(arith_op, result_reg, rk_left, rk_right, self.current_line);
        ValueResult::make_register(result_reg, true, AccessKind::None)
    }

    /// 常量算术折叠
    fn fold_arithmetic(op: BinaryOp, left: f64, right: f64) -> Option<f64> {
        let result = match op {
            BinaryOp::Add => left + right,
            BinaryOp::Sub => left - right,
            BinaryOp::Mul => left * right,
            BinaryOp::Div => {
                if right == 0.0 {
                    return None;
                }
                left / right
            }
            BinaryOp::Mod => {
                if right == 0.0 {
                    return None;
                }
                left - (left / right).floor() * right
            }
            BinaryOp::Pow => left.powf(right),
            _ => return None,
        };
        if result.is_nan() { None } else { Some(result) }
    }

    // ═══════════════════════════════════════════════════════════════
    // 一元表达式 lowering
    // ═══════════════════════════════════════════════════════════════

    fn emit_value_unary(&mut self, e: &UnaryExpr) -> ValueResult {
        match e.op {
            UnaryOp::Not => {
                let cond = CondResult {
                    true_list: self.emit_cond_result(&e.operand).false_list,
                    ..Default::default()
                };
                let result_reg = self.reg_alloc.alloc();
                self.materialize_cond_result(&cond, result_reg, false);
                ValueResult::make_register(result_reg, true, AccessKind::None)
            }
            UnaryOp::Neg => {
                let operand = self.emit_value(&e.operand);
                if let ValueResult::Immediate {
                    kind: ImmediateKind::Number,
                    number_value,
                    ..
                } = &operand
                {
                    return ValueResult::make_number(-number_value);
                }
                let op_reg = self.value_to_any_reg(operand);
                self.free_reg(op_reg);
                let pc = self.code_abc(OpCode::UNM, 0, op_reg, 0, self.current_line);
                ValueResult::make_relocatable(pc)
            }
            UnaryOp::Len => {
                let operand = self.emit_value(&e.operand);
                let op_reg = self.value_to_any_reg(operand);
                self.free_reg(op_reg);
                let pc = self.code_abc(OpCode::LEN, 0, op_reg, 0, self.current_line);
                ValueResult::make_relocatable(pc)
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // 表构造器
    // ═══════════════════════════════════════════════════════════════

    fn emit_value_table(&mut self, table: &TableExpr) -> ValueResult {
        let pc = self.code_abc(OpCode::NEWTABLE, 0, 0, 0, self.current_line);
        let table_reg = self.reg_alloc.alloc();
        let mut inst = self.builder.instruction(pc).unwrap();
        opcode::set_arg_a(&mut inst, table_reg);
        self.builder.replace_instruction(pc, inst);

        let mut array_count = 0;
        let mut hash_count = 0;
        let mut pending_array = 0;

        for (field_index, field) in table.fields.iter().enumerate() {
            if let Some(key_expr) = &field.key {
                let saved_freereg = table_reg + pending_array + 1;

                let key = self.emit_value(key_expr);
                let rk_key = self.value_to_rk(key);
                let value = self.emit_value(&field.value);
                let value = self.force_single_value(value);
                let rk_value = self.value_to_rk(value);
                self.code_abc(
                    OpCode::SETTABLE,
                    table_reg,
                    rk_key,
                    rk_value,
                    self.current_line,
                );

                self.reg_alloc.set_freereg(saved_freereg);
                hash_count += 1;
            } else {
                array_count += 1;
                pending_array += 1;

                let target_reg = table_reg + pending_array;
                self.reg_alloc.ensure_at_least(target_reg + 1);

                let is_last_field = field_index + 1 == table.fields.len();
                if is_last_field && let Expr::Call(call) = field.value.as_ref() {
                    let mut info = self.emit_call_expr(call, target_reg);
                    self.set_open_multi_ret(&mut info);
                    let block = (array_count - 1) / LFIELDS_PER_FLUSH + 1;
                    self.emit_setlist(table_reg, 0, block);
                    self.reg_alloc.set_freereg(table_reg + 1);
                    pending_array = 0;
                    continue;
                }

                if is_last_field && matches!(field.value.as_ref(), Expr::Vararg(_)) {
                    let mut info = self.emit_vararg_expr();
                    self.set_multi_ret_base(&mut info, target_reg);
                    self.set_open_multi_ret(&mut info);
                    let block = (array_count - 1) / LFIELDS_PER_FLUSH + 1;
                    self.emit_setlist(table_reg, 0, block);
                    self.reg_alloc.set_freereg(table_reg + 1);
                    pending_array = 0;
                    continue;
                }

                let value = self.emit_value(&field.value);
                let value = self.force_single_value(value);
                self.materialize_value(value, target_reg);
                self.reg_alloc.ensure_at_least(target_reg + 1);

                if pending_array == LFIELDS_PER_FLUSH {
                    let block = (array_count - 1) / LFIELDS_PER_FLUSH + 1;
                    self.emit_setlist(table_reg, pending_array, block);
                    self.reg_alloc.set_freereg(table_reg + 1);
                    pending_array = 0;
                }
            }
        }

        if pending_array > 0 {
            let block = (array_count - 1) / LFIELDS_PER_FLUSH + 1;
            self.emit_setlist(table_reg, pending_array, block);
            self.reg_alloc.set_freereg(table_reg + 1);
        }

        let mut inst = self.builder.instruction(pc).unwrap();
        opcode::set_arg_b(&mut inst, array_count.min(MAXARG_C));
        opcode::set_arg_c(&mut inst, hash_count.min(MAXARG_C));
        self.builder.replace_instruction(pc, inst);

        ValueResult::make_register(table_reg, true, AccessKind::None)
    }

    fn emit_setlist(&mut self, table_reg: i32, count: i32, block: i32) {
        if block <= MAXARG_C {
            self.code_abc(OpCode::SETLIST, table_reg, count, block, self.current_line);
        } else {
            self.code_abc(OpCode::SETLIST, table_reg, count, 0, self.current_line);
            self.builder.emit_raw(self.current_line, block as u32);
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // 索引 / 成员访问
    // ═══════════════════════════════════════════════════════════════

    fn emit_value_index(&mut self, e: &IndexExpr) -> ValueResult {
        let table = self.emit_value(&e.table);
        let table_reg = self.value_to_any_reg(table);
        let key = self.emit_value(&e.index);
        let rk_key = self.value_to_rk(key);
        ValueResult::make_pending_load(AccessKind::Indexed, table_reg, -1, rk_key)
    }

    fn emit_value_member(&mut self, e: &MemberExpr) -> ValueResult {
        let table = self.emit_value(&e.table);
        let table_reg = self.value_to_any_reg(table);
        // SAFETY: self.gc is set during CodeGenerator::new() from a valid &mut GC
        let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
        let k = self
            .builder
            .add_string_constant(gc, &e.member)
            .unwrap_or_else(|| self.builder.add_number_constant(0.0));
        let rk_key = self.value_to_rk(ValueResult::make_constant(k));
        ValueResult::make_pending_load(AccessKind::Indexed, table_reg, -1, rk_key)
    }

    // ═══════════════════════════════════════════════════════════════
    // 函数表达式 → CLOSURE
    // ═══════════════════════════════════════════════════════════════

    fn emit_value_function_expr(&mut self, e: &FunctionExpr) -> ValueResult {
        let linedefined = e.location.line;
        let lastlinedefined = if e.end_line > 0 {
            e.end_line
        } else {
            linedefined
        };
        let function = self
            .compile_function(
                &e.params,
                e.is_vararg,
                &e.body,
                linedefined,
                lastlinedefined.max(linedefined),
            )
            .expect("function expression compilation should succeed");
        let reg = self.reg_alloc.alloc();
        self.code_abx(
            OpCode::CLOSURE,
            reg,
            function.proto_index,
            self.current_line,
        );
        self.emit_closure_upvalues(&function.upvalues);
        ValueResult::make_register(reg, true, AccessKind::None)
    }

    // ═══════════════════════════════════════════════════════════════
    // 调用表达式
    // ═══════════════════════════════════════════════════════════════

    pub fn emit_call_expr(&mut self, e: &CallExpr, _target_base: i32) -> CallResultInfo {
        let (base, first_arg_reg, implicit_args) = if e.is_method_call {
            let Expr::Member(member) = e.func.as_ref() else {
                panic!("method call must use a member expression as callee");
            };
            let obj = self.emit_value(&member.table);
            let obj_reg = self.value_to_any_reg(obj);

            // SAFETY: self.gc is set during CodeGenerator::new() from a valid &mut GC.
            let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
            let method_key = self
                .builder
                .add_string_constant(gc, &member.member)
                .unwrap_or_else(|| self.builder.add_number_constant(0.0));
            let rk_key = if method_key <= MAXINDEXRK {
                rk_ask(method_key)
            } else {
                self.value_to_any_reg(ValueResult::make_constant(method_key))
            };

            let base = if _target_base >= 0 {
                _target_base
            } else {
                self.reg_alloc.current()
            };
            self.reg_alloc.ensure_at_least(base + 2);
            self.code_abc(OpCode::SELF, base, obj_reg, rk_key, self.current_line);
            (base, base + 2, 1)
        } else {
            let func_val = self.emit_value(&e.func);
            let base = if _target_base >= 0 {
                self.materialize_value(func_val, _target_base);
                self.reg_alloc.ensure_at_least(_target_base + 1);
                _target_base
            } else {
                match &func_val {
                    ValueResult::RegisterRef {
                        reg,
                        owns_register: true,
                        ..
                    } if *reg >= self.active_var_count && *reg == self.reg_alloc.current() - 1 => {
                        *reg
                    }
                    _ => {
                        let base = self.reg_alloc.current();
                        self.materialize_value(func_val, base);
                        self.reg_alloc.ensure_at_least(base + 1);
                        base
                    }
                }
            };
            (base, base + 1, 0)
        };

        let saved_free_reg = self.reg_alloc.current();
        self.reg_alloc.set_freereg(first_arg_reg);
        let _ = self
            .reg_alloc
            .check_stack(e.args.len() as i32, self.builder.max_stack_size() as i32);

        let nargs = e.args.len() as i32;
        let mut last_arg_is_multi_ret = false;
        for (i, arg) in e.args.iter().enumerate() {
            let target_reg = first_arg_reg + i as i32;
            let is_last_arg = i + 1 == e.args.len();
            if is_last_arg && let Expr::Call(call) = arg.as_ref() {
                let mut info = self.emit_call_expr(call, target_reg);
                self.set_open_multi_ret(&mut info);
                last_arg_is_multi_ret = true;
            } else if is_last_arg && matches!(arg.as_ref(), Expr::Vararg(_)) {
                let mut info = self.emit_vararg_expr();
                self.set_multi_ret_base(&mut info, target_reg);
                self.set_open_multi_ret(&mut info);
                last_arg_is_multi_ret = true;
            } else {
                let arg_val = self.emit_value(arg);
                let arg_val = self.force_single_value(arg_val);
                self.materialize_value(arg_val, target_reg);
            }
            self.reg_alloc.ensure_at_least(target_reg + 1);
        }

        let b_arg = if last_arg_is_multi_ret {
            0
        } else {
            nargs + implicit_args + 1
        };
        let call_pc = self.code_abc(OpCode::CALL, base, b_arg, 2, self.current_line); // C=2 → 1 return

        self.reg_alloc.set_freereg(saved_free_reg.max(base + 1));

        CallResultInfo {
            kind: CallResultKind::Call,
            base_reg: base,
            instruction_pc: call_pc,
            open_multi_ret: false,
        }
    }

    /// Vararg 表达式
    pub fn emit_vararg_expr(&mut self) -> CallResultInfo {
        let pc = self.code_abc(OpCode::VARARG, 0, 1, 0, self.current_line);
        CallResultInfo {
            kind: CallResultKind::Vararg,
            base_reg: -1,
            instruction_pc: pc,
            open_multi_ret: false,
        }
    }

    pub fn set_open_multi_ret(&mut self, info: &mut CallResultInfo) {
        let mut inst = self.builder.instruction(info.instruction_pc).unwrap();
        match info.kind {
            CallResultKind::Call => opcode::set_arg_c(&mut inst, 0),
            CallResultKind::Vararg => opcode::set_arg_b(&mut inst, 0),
            CallResultKind::None => {}
        }
        self.builder.replace_instruction(info.instruction_pc, inst);
        info.open_multi_ret = true;
    }

    pub fn set_wanted_results(&mut self, info: &mut CallResultInfo, wanted: i32) {
        let mut inst = self.builder.instruction(info.instruction_pc).unwrap();
        match info.kind {
            CallResultKind::Call => opcode::set_arg_c(&mut inst, wanted + 1),
            CallResultKind::Vararg => opcode::set_arg_b(&mut inst, wanted + 1),
            CallResultKind::None => {}
        }
        self.builder.replace_instruction(info.instruction_pc, inst);
        info.open_multi_ret = false;
    }

    pub fn set_multi_ret_base(&mut self, info: &mut CallResultInfo, base: i32) {
        if info.kind == CallResultKind::Vararg {
            let mut inst = self.builder.instruction(info.instruction_pc).unwrap();
            opcode::set_arg_a(&mut inst, base);
            self.builder.replace_instruction(info.instruction_pc, inst);
            info.base_reg = base;
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // 比较跳转
    // ═══════════════════════════════════════════════════════════════

    fn emit_comparison_jump(&mut self, e: &BinaryExpr, jump_on_true: bool) -> PatchList {
        let (cmp_op, cond, swap) = match e.op {
            BinaryOp::Eq => (OpCode::EQ, if jump_on_true { 1 } else { 0 }, false),
            BinaryOp::Ne => (OpCode::EQ, if jump_on_true { 0 } else { 1 }, false),
            BinaryOp::Lt => (OpCode::LT, if jump_on_true { 1 } else { 0 }, false),
            BinaryOp::Le => (OpCode::LE, if jump_on_true { 1 } else { 0 }, false),
            BinaryOp::Gt => (OpCode::LT, if jump_on_true { 1 } else { 0 }, true),
            BinaryOp::Ge => (OpCode::LE, if jump_on_true { 1 } else { 0 }, true),
            _ => return PatchList::new(),
        };

        let left = self.emit_value(&e.left);
        let mut o1 = self.value_to_rk(left);
        let right = self.emit_value(&e.right);
        let mut o2 = self.value_to_rk(right);

        if swap {
            std::mem::swap(&mut o1, &mut o2);
        }

        if o1 > o2 {
            self.free_reg(o1);
            self.free_reg(o2);
        } else {
            self.free_reg(o2);
            self.free_reg(o1);
        }

        self.code_abc(cmp_op, cond, o1, o2, self.current_line);
        let mut result = PatchList::new();
        result.append(self.emit_jump());
        result
    }

    fn materialize_cond_result(&mut self, cond: &CondResult, reg: i32, _fallthrough_on_true: bool) {
        self.code_abc(OpCode::LOADBOOL, reg, 0, 1, self.current_line);
        let label = self.get_label();
        self.patch_list_vec(&cond.true_list, label);
        self.code_abc(OpCode::LOADBOOL, reg, 1, 0, self.current_line);
    }

    // ═══════════════════════════════════════════════════════════════
    // 左值 / 存储
    // ═══════════════════════════════════════════════════════════════

    /// 发射左值
    pub fn emit_lvalue(&mut self, expr: &Expr) -> Result<LValueRef, String> {
        match expr {
            Expr::Name(n) => {
                let sym = self.resolve_name(&n.name);
                Ok(self.symbol_to_lvalue(&sym))
            }
            Expr::Index(i) => {
                let table_val = self.emit_value(&i.table);
                let table_reg = self.value_to_any_reg(table_val);
                let key_val = self.emit_value(&i.index);
                let rk_key = self.value_to_rk(key_val);
                let mut result = LValueRef::new();
                result.kind = LValueKind::Indexed;
                result.table_reg = table_reg;
                result.key = rk_key;
                Ok(result)
            }
            Expr::Member(m) => {
                let table_val = self.emit_value(&m.table);
                let table_reg = self.value_to_any_reg(table_val);
                // SAFETY: self.gc is set from a valid &mut GC during CodeGenerator::new()
                let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
                let k = self
                    .builder
                    .add_string_constant(gc, &m.member)
                    .unwrap_or_else(|| self.builder.add_number_constant(0.0));
                let rk_key = self.value_to_rk(ValueResult::make_constant(k));
                let mut result = LValueRef::new();
                result.kind = LValueKind::Indexed;
                result.table_reg = table_reg;
                result.key = rk_key;
                Ok(result)
            }
            _ => Err("Expression is not a valid lvalue".to_string()),
        }
    }

    /// 将值存储到左值目标
    pub fn emit_store(&mut self, target: &LValueRef, val: ValueResult) {
        match target.kind {
            LValueKind::Local => {
                self.materialize_value(val, target.slot);
            }
            LValueKind::Upvalue => {
                let v = self.force_single_value(val);
                let reg = self.value_to_any_reg(v);
                self.code_abc(OpCode::SETUPVAL, reg, target.slot, 0, self.current_line);
            }
            LValueKind::Global => {
                let v = self.force_single_value(val);
                let reg = self.value_to_any_reg(v);
                self.code_abx(OpCode::SETGLOBAL, reg, target.slot, self.current_line);
            }
            LValueKind::Indexed => {
                let v = self.force_single_value(val);
                let rk = self.value_to_rk(v);
                self.code_abc(
                    OpCode::SETTABLE,
                    target.table_reg,
                    target.key,
                    rk,
                    self.current_line,
                );
            }
            LValueKind::None => {}
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // 辅助
    // ═══════════════════════════════════════════════════════════════

    /// 判断常量真值性
    fn constant_truthiness(&self, val: &ValueResult) -> PayloadTruthiness {
        match val {
            ValueResult::Immediate {
                kind,
                boolean_value,
                ..
            } => match kind {
                ImmediateKind::Nil => PayloadTruthiness::Falsy,
                ImmediateKind::Boolean => {
                    if *boolean_value {
                        PayloadTruthiness::Truthy
                    } else {
                        PayloadTruthiness::Falsy
                    }
                }
                ImmediateKind::Number => PayloadTruthiness::Truthy,
                ImmediateKind::None => PayloadTruthiness::Runtime,
            },
            ValueResult::ConstantRef { .. } => PayloadTruthiness::Truthy,
            _ => PayloadTruthiness::Runtime,
        }
    }

    fn free_reg(&mut self, reg: i32) {
        self.reg_alloc.free_reg(reg, self.active_var_count);
    }
}

enum PayloadTruthiness {
    Falsy,
    Truthy,
    Runtime,
}
