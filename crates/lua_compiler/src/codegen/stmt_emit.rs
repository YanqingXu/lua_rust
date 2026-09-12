//! 语句发射器 (Statement Emitter)
//!
//! 实现 AST 语句 → 字节码的 lowering：赋值、局部声明、控制流、
//! 循环、return、函数定义等全部 13 种 Stmt 节点。
//!

use std::collections::HashSet;

use crate::ast::expr::Expr;
use crate::ast::stmt::{
    AssignStmt, CallStmt, DoStmt, ForInStmt, ForNumStmt, FunctionStmt, IfStmt, LocalStmt,
    RepeatStmt, ReturnStmt, Stmt, WhileStmt,
};
use crate::codegen::types::{AccessKind, LValueKind, NO_JUMP, SymbolKind, ValueResult};
use crate::codegen::{CodeGenerator, CodegenObjectAllocator};
use crate::opcode::{MAXINDEXRK, OpCode, is_k, rk_ask};

fn expression_has_call(expr: &Expr) -> bool {
    match expr {
        Expr::Nil(_)
        | Expr::Boolean(_)
        | Expr::Number(_)
        | Expr::String(_)
        | Expr::Vararg(_)
        | Expr::Name(_)
        | Expr::Function(_) => false,
        Expr::Binary(binary) => {
            expression_has_call(&binary.left) || expression_has_call(&binary.right)
        }
        Expr::Unary(unary) => expression_has_call(&unary.operand),
        Expr::Call(_) => true,
        Expr::Table(table) => table.fields.iter().any(|field| {
            field.key.as_deref().is_some_and(expression_has_call)
                || expression_has_call(&field.value)
        }),
        Expr::Index(index) => {
            expression_has_call(&index.table) || expression_has_call(&index.index)
        }
        Expr::Member(member) => expression_has_call(&member.table),
        Expr::Paren(paren) => expression_has_call(&paren.expression),
    }
}

fn collect_expr_reads(expr: &Expr, reads: &mut HashSet<String>) {
    match expr {
        Expr::Name(name) => {
            reads.insert(name.name.clone());
        }
        Expr::Binary(binary) => {
            collect_expr_reads(&binary.left, reads);
            collect_expr_reads(&binary.right, reads);
        }
        Expr::Unary(unary) => collect_expr_reads(&unary.operand, reads),
        Expr::Table(table) => {
            for field in &table.fields {
                if let Some(key) = &field.key {
                    collect_expr_reads(key, reads);
                }
                collect_expr_reads(&field.value, reads);
            }
        }
        Expr::Call(call) => {
            collect_expr_reads(&call.func, reads);
            for arg in &call.args {
                collect_expr_reads(arg, reads);
            }
        }
        Expr::Index(index) => {
            collect_expr_reads(&index.table, reads);
            collect_expr_reads(&index.index, reads);
        }
        Expr::Member(member) => collect_expr_reads(&member.table, reads),
        Expr::Function(function) => collect_stmt_list_reads(&function.body, reads),
        Expr::Paren(paren) => collect_expr_reads(&paren.expression, reads),
        Expr::Nil(_) | Expr::Boolean(_) | Expr::Number(_) | Expr::String(_) | Expr::Vararg(_) => {}
    }
}

fn collect_assign_target_reads(target: &Expr, reads: &mut HashSet<String>) {
    if !matches!(target, Expr::Name(_)) {
        collect_expr_reads(target, reads);
    }
}

fn collect_stmt_reads(stmt: &Stmt, reads: &mut HashSet<String>) {
    match stmt {
        Stmt::Assign(assign) => {
            for target in &assign.targets {
                collect_assign_target_reads(target, reads);
            }
            for value in &assign.values {
                collect_expr_reads(value, reads);
            }
        }
        Stmt::Local(local) => {
            for value in &local.values {
                collect_expr_reads(value, reads);
            }
        }
        Stmt::Call(call) => collect_expr_reads(&call.call, reads),
        Stmt::If(if_stmt) => {
            for branch in &if_stmt.branches {
                collect_expr_reads(&branch.condition, reads);
                collect_stmt_list_reads(&branch.body, reads);
            }
            collect_stmt_list_reads(&if_stmt.else_branch, reads);
        }
        Stmt::While(while_stmt) => {
            collect_expr_reads(&while_stmt.condition, reads);
            collect_stmt_list_reads(&while_stmt.body, reads);
        }
        Stmt::Repeat(repeat_stmt) => {
            collect_stmt_list_reads(&repeat_stmt.body, reads);
            collect_expr_reads(&repeat_stmt.condition, reads);
        }
        Stmt::ForNum(for_stmt) => {
            collect_expr_reads(&for_stmt.init, reads);
            collect_expr_reads(&for_stmt.limit, reads);
            if let Some(step) = &for_stmt.step {
                collect_expr_reads(step, reads);
            }
            collect_stmt_list_reads(&for_stmt.body, reads);
        }
        Stmt::ForIn(for_stmt) => {
            for iterator in &for_stmt.iterators {
                collect_expr_reads(iterator, reads);
            }
            collect_stmt_list_reads(&for_stmt.body, reads);
        }
        Stmt::Function(function) => {
            if !function.is_local {
                reads.insert(function.name.clone());
            }
            collect_stmt_list_reads(&function.body, reads);
        }
        Stmt::Return(return_stmt) => {
            for value in &return_stmt.values {
                collect_expr_reads(value, reads);
            }
        }
        Stmt::Do(do_stmt) => collect_stmt_list_reads(&do_stmt.body, reads),
        Stmt::Empty(_) | Stmt::Break(_) => {}
    }
}

fn collect_stmt_list_reads(stmts: &[Box<Stmt>], reads: &mut HashSet<String>) {
    for stmt in stmts {
        collect_stmt_reads(stmt, reads);
    }
}

fn receiver_table_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Index(index) => match index.table.as_ref() {
            Expr::Name(name) => Some(name.name.as_str()),
            _ => None,
        },
        Expr::Member(member) => match member.table.as_ref() {
            Expr::Name(name) => Some(name.name.as_str()),
            _ => None,
        },
        _ => None,
    }
}

impl<A: CodegenObjectAllocator + ?Sized> CodeGenerator<'_, A> {
    // ═══════════════════════════════════════════════════════════════
    // 公开入口 — 语句 / 语句块
    // ═══════════════════════════════════════════════════════════════

    /// 发射单条语句
    pub fn emit_statement(&mut self, stmt: &Stmt) -> Result<(), String> {
        self.check_codegen_error()?;
        match stmt {
            Stmt::Empty(_) => {}
            Stmt::Assign(a) => self.emit_assign(a)?,
            Stmt::Local(l) => self.emit_local(l),
            Stmt::Call(c) => self.emit_call_stmt(c)?,
            Stmt::If(i) => self.emit_if(i)?,
            Stmt::While(w) => self.emit_while(w)?,
            Stmt::Repeat(r) => self.emit_repeat(r)?,
            Stmt::ForNum(f) => self.emit_for_num(f)?,
            Stmt::ForIn(f) => self.emit_for_in(f)?,
            Stmt::Function(f) => self.emit_function_stmt(f)?,
            Stmt::Return(r) => self.emit_return(r),
            Stmt::Break(b) => {
                self.current_line = b.location.line;
                self.emit_break();
            }
            Stmt::Do(d) => self.emit_do(d)?,
        }
        self.reg_alloc.reset_to_locals(self.active_var_count);
        self.check_codegen_error()
    }

    /// 发射语句块
    pub fn emit_block(&mut self, stmts: &[Box<Stmt>]) -> Result<(), String> {
        self.check_codegen_error()?;
        let block_level = self.active_var_count;
        let block_line = self.current_line;
        let mut reads_after = vec![HashSet::new(); stmts.len() + 1];
        for index in (0..stmts.len()).rev() {
            let mut reads = reads_after[index + 1].clone();
            collect_stmt_reads(&stmts[index], &mut reads);
            reads_after[index] = reads;
        }

        let previous_future_reads = self.future_reads.take();
        for (index, stmt) in stmts.iter().enumerate() {
            self.future_reads = Some(reads_after[index + 1].clone());
            if let Err(error) = self.emit_statement(stmt) {
                self.future_reads = previous_future_reads;
                self.current_line = block_line;
                return Err(error);
            }
            self.current_line = block_line;
        }
        self.future_reads = previous_future_reads;
        self.remove_local_vars(block_level);
        self.check_codegen_error()
    }

    // ═══════════════════════════════════════════════════════════════
    // 赋值语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_assign(&mut self, a: &AssignStmt) -> Result<(), String> {
        self.current_line = a.location.line;
        let nvars = a.targets.len() as i32;
        let nexps = a.values.len() as i32;

        if nvars == 0 {
            return Ok(());
        }

        if nvars == 1
            && nexps == 1
            && let (Expr::Name(target), Expr::Name(value)) =
                (a.targets[0].as_ref(), a.values[0].as_ref())
            && target.name == value.name
            && self.find_local_var(&target.name) >= 0
        {
            return Ok(());
        }

        if nvars == nexps {
            let local_assignment = a
                .targets
                .iter()
                .zip(&a.values)
                .map(|(target, value)| match (target.as_ref(), value.as_ref()) {
                    (Expr::Name(target), Expr::Name(value)) => {
                        let target_reg = self.find_local_var(&target.name);
                        let source_reg = self.find_local_var(&value.name);
                        (target_reg >= 0 && source_reg >= 0).then_some((target_reg, source_reg))
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>();

            if let Some(registers) = local_assignment {
                let changed = registers.iter().any(|(target, source)| target != source);
                if !changed {
                    return Ok(());
                }

                if registers.len() == 2
                    && registers[0].0 == registers[1].1
                    && registers[1].0 == registers[0].1
                {
                    let temp_reg = self.reg_alloc.alloc();
                    self.code_abc(OpCode::MOVE, temp_reg, registers[0].1, 0, self.current_line);
                    self.code_abc(
                        OpCode::MOVE,
                        registers[0].0,
                        registers[1].1,
                        0,
                        self.current_line,
                    );
                    self.code_abc(OpCode::MOVE, registers[1].0, temp_reg, 0, self.current_line);
                    self.reg_alloc.set_freereg(temp_reg);
                    return Ok(());
                }

                let has_forward_dependency = (0..registers.len())
                    .any(|i| ((i + 1)..registers.len()).any(|j| registers[i].0 == registers[j].1));
                if !has_forward_dependency {
                    for (target, source) in registers {
                        if target != source {
                            self.code_abc(OpCode::MOVE, target, source, 0, self.current_line);
                        }
                    }
                    return Ok(());
                }
            }
        }

        let rhs_has_call = a.values.iter().any(|value| expression_has_call(value));
        let assigned_local_names = a
            .targets
            .iter()
            .filter_map(|target| match target.as_ref() {
                Expr::Name(name) if self.find_local_var(&name.name) >= 0 => {
                    Some(name.name.as_str())
                }
                _ => None,
            })
            .collect::<HashSet<_>>();

        let mut targets = Vec::with_capacity(a.targets.len());
        for target_expr in &a.targets {
            let mut target = self.emit_lvalue(target_expr)?;
            if target.kind == LValueKind::Indexed {
                let stable_receiver = receiver_table_name(target_expr).is_some_and(|name| {
                    self.find_local_var(name) >= 0 && !assigned_local_names.contains(name)
                });
                if rhs_has_call || !stable_receiver {
                    let stable_reg = self.reg_alloc.alloc();
                    if stable_reg != target.table_reg {
                        self.code_abc(
                            OpCode::MOVE,
                            stable_reg,
                            target.table_reg,
                            0,
                            self.current_line,
                        );
                    }
                    target.table_reg = stable_reg;
                }

                let key_expr = match target_expr.as_ref() {
                    Expr::Index(index) => Some(index.index.as_ref()),
                    _ => None,
                };
                let stable_key = is_k(target.key)
                    || (!rhs_has_call
                        && key_expr.is_some_and(|expr| match expr {
                            Expr::Name(name) => {
                                self.find_local_var(&name.name) >= 0
                                    && !assigned_local_names.contains(name.name.as_str())
                            }
                            _ => false,
                        }));
                if !stable_key {
                    let stable_reg = self.reg_alloc.alloc();
                    if stable_reg != target.key {
                        self.code_abc(OpCode::MOVE, stable_reg, target.key, 0, self.current_line);
                    }
                    target.key = stable_reg;
                }
            }
            targets.push(target);
        }

        let value_base = self.reg_alloc.current();
        let mut assigned_values = 0;
        let direct_values = nvars.min(nexps);
        let assignment_needs_frozen_values = nvars > 1 || nexps > 1;
        let mut values_for_store = Vec::with_capacity(a.targets.len());

        for i in 0..direct_values {
            let expr = &a.values[i as usize];
            let is_last_expr = i == nexps - 1;
            let target_reg = value_base + i;

            if is_last_expr && nexps <= nvars {
                let wanted = nvars - i;
                if let Expr::Call(call) = expr.as_ref() {
                    let mut info = self.emit_call_expr(call, target_reg);
                    self.set_wanted_results(&mut info, wanted);
                    self.reg_alloc.ensure_at_least(value_base + nvars);
                    assigned_values = nvars;
                    break;
                }
                if matches!(expr.as_ref(), Expr::Vararg(_)) {
                    let mut info = self.emit_vararg_expr();
                    self.set_multi_ret_base(&mut info, target_reg);
                    self.set_wanted_results(&mut info, wanted);
                    self.reg_alloc.ensure_at_least(value_base + nvars);
                    assigned_values = nvars;
                    break;
                }
            }

            self.reg_alloc.set_freereg(target_reg);
            let val = self.emit_value(expr);
            let val = self.force_single_value(val);
            let borrows_stable_local = i == direct_values - 1
                && matches!(expr.as_ref(), Expr::Name(name) if self.find_local_var(&name.name) >= 0);
            if assignment_needs_frozen_values && !borrows_stable_local {
                self.materialize_value(val, target_reg);
                values_for_store.push(ValueResult::make_register(
                    target_reg,
                    false,
                    AccessKind::None,
                ));
            } else {
                values_for_store.push(val);
            }
            self.reg_alloc.set_freereg(target_reg + 1);
            assigned_values = i + 1;
        }

        for i in direct_values..nexps {
            let expr = &a.values[i as usize];
            let scratch_reg = value_base + nvars;
            self.reg_alloc.set_freereg(scratch_reg);
            if let Expr::Call(call) = expr.as_ref() {
                let mut info = self.emit_call_expr(call, scratch_reg);
                self.set_wanted_results(&mut info, 0);
            } else if matches!(expr.as_ref(), Expr::Vararg(_)) {
                let mut info = self.emit_vararg_expr();
                self.set_multi_ret_base(&mut info, scratch_reg);
                self.set_wanted_results(&mut info, 0);
            } else {
                let val = self.emit_value(expr);
                let val = self.force_single_value(val);
                self.materialize_value(val, scratch_reg);
            }
            self.reg_alloc.set_freereg(scratch_reg);
        }

        for (i, target) in targets.iter().enumerate().rev() {
            let val = if (i as i32) < assigned_values {
                values_for_store.get(i).cloned().unwrap_or_else(|| {
                    ValueResult::make_register(value_base + i as i32, false, AccessKind::None)
                })
            } else {
                ValueResult::make_nil()
            };
            self.emit_store(target, val);
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // 局部变量声明
    // ═══════════════════════════════════════════════════════════════

    fn emit_local(&mut self, l: &LocalStmt) {
        self.current_line = l.location.line;
        let nvars = l.names.len() as i32;
        let nexps = l.values.len() as i32;
        let base = self.active_var_count;

        self.reg_alloc.set_freereg(base);

        let all_nil_initializers = !l.values.is_empty()
            && l.values
                .iter()
                .all(|value| matches!(value.as_ref(), Expr::Nil(_)));
        let mut all_vars_initialized = nexps == 0
            && nvars > 0
            && !self.force_nil_local_initialization
            && self.builder.instruction_count() == 0;

        if !all_vars_initialized
            && nexps == 0
            && nvars > 0
            && !self.force_nil_local_initialization
            && let Some(future_reads) = &self.future_reads
        {
            all_vars_initialized = l.names.iter().all(|name| !future_reads.contains(name));
        }

        let mut initialized = 0;
        if all_nil_initializers {
            if nvars > 0 {
                self.code_abc(
                    OpCode::LOADNIL,
                    base,
                    base + nvars - 1,
                    0,
                    self.current_line,
                );
                self.reg_alloc.ensure_at_least(base + nvars);
                initialized = nvars;
                all_vars_initialized = true;
            }
        } else {
            let prefix_count = if nexps <= nvars {
                nexps.saturating_sub(1)
            } else {
                nvars
            };

            for (i, value_expr) in l.values.iter().take(prefix_count as usize).enumerate() {
                let target_reg = base + i as i32;
                let val = self.emit_value(value_expr);
                let val = self.force_single_value(val);
                self.materialize_value(val, target_reg);
                self.reg_alloc.ensure_at_least(target_reg + 1);
                initialized += 1;
            }

            if nexps > 0 && nexps <= nvars {
                let last_index = (nexps - 1) as usize;
                let target_reg = base + nexps - 1;
                let wanted = nvars - (nexps - 1);
                if let Expr::Call(call) = l.values[last_index].as_ref() {
                    let mut info = self.emit_call_expr(call, -1);
                    self.set_wanted_results(&mut info, wanted);
                    self.move_reg_range(target_reg, info.base_reg, wanted);
                    self.reg_alloc.ensure_at_least(base + nvars);
                    initialized = nvars;
                } else if matches!(l.values[last_index].as_ref(), Expr::Vararg(_)) {
                    let mut info = self.emit_vararg_expr();
                    self.set_multi_ret_base(&mut info, target_reg);
                    self.set_wanted_results(&mut info, wanted);
                    self.reg_alloc.ensure_at_least(base + nvars);
                    initialized = nvars;
                } else {
                    let val = self.emit_value(&l.values[last_index]);
                    let val = self.force_single_value(val);
                    self.materialize_value(val, target_reg);
                    self.reg_alloc.ensure_at_least(target_reg + 1);
                    initialized += 1;
                }
            }
        }

        if initialized < nvars && !all_vars_initialized {
            let loadnil_line = if nexps == 0 { 0 } else { self.current_line };
            self.code_abc(
                OpCode::LOADNIL,
                base + initialized,
                base + nvars - 1,
                0,
                loadnil_line,
            );
        }

        self.reg_alloc.set_freereg(base);
        for name in &l.names {
            self.add_local_var(name.clone());
        }

        self.adjust_local_vars(nvars);
    }

    // ═══════════════════════════════════════════════════════════════
    // 函数调用语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_call_stmt(&mut self, c: &CallStmt) -> Result<(), String> {
        self.current_line = c.location.line;
        if let Expr::Call(call) = c.call.as_ref() {
            let mut info = self.emit_call_expr(call, -1);
            self.set_wanted_results(&mut info, 0);
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // If 语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_if(&mut self, i: &IfStmt) -> Result<(), String> {
        self.current_line = i.location.line;
        let mut exit_list: Vec<i32> = Vec::new();

        for (idx, branch) in i.branches.iter().enumerate() {
            self.check_codegen_error()?;
            self.current_line = branch.condition.line();
            let cond = self.emit_cond_result(&branch.condition);
            self.check_codegen_error()?;
            self.patch_list_to_here(&cond.true_list);

            self.enter_block(false);
            self.emit_block(&branch.body)?;
            self.current_line = i.location.line;
            self.leave_block();

            if idx < i.branches.len() - 1 || !i.else_branch.is_empty() {
                let saved_line = self.current_line;
                self.current_line = i.end_line;
                exit_list.push(self.emit_jump());
                self.current_line = saved_line;
            }
            self.patch_list_to_here(&cond.false_list);
        }

        if !i.else_branch.is_empty() {
            self.enter_block(false);
            self.emit_block(&i.else_branch)?;
            self.current_line = i.location.line;
            self.leave_block();
        }

        let target = self.get_label();
        for pc in exit_list {
            self.fix_jump(pc, target);
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // While 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_while(&mut self, w: &WhileStmt) -> Result<(), String> {
        self.current_line = w.location.line;
        self.enter_block(true);

        let loop_start = self.get_label();
        let cond = self.emit_cond_result(&w.condition);
        self.check_codegen_error()?;
        self.patch_list_to_here(&cond.true_list);

        let body_level = self.active_var_count;
        self.emit_block(&w.body)?;
        self.current_line = w.location.line;
        self.remove_local_vars(body_level);

        // 跳回循环头
        let offset = loop_start - (self.get_label() + 1);
        self.builder.emit_as_bx(0, OpCode::JMP, 0, offset);

        self.patch_list_to_here(&cond.false_list);
        self.leave_block();
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // Repeat 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_repeat(&mut self, r: &RepeatStmt) -> Result<(), String> {
        self.current_line = r.location.line;
        let block_level = self.active_var_count;
        self.enter_block(true);

        let loop_start = self.get_label();
        let saved_force_nil_local_initialization = self.force_nil_local_initialization;
        self.force_nil_local_initialization = true;
        let body_result: Result<(), String> = r.body.iter().try_for_each(|stmt| {
            self.emit_statement(stmt)?;
            self.current_line = r.location.line;
            Ok(())
        });
        self.force_nil_local_initialization = saved_force_nil_local_initialization;
        body_result?;

        if matches!(r.condition.as_ref(), Expr::Boolean(boolean) if boolean.value) {
            self.remove_local_vars(block_level);
            self.leave_block();
            return Ok(());
        }

        // The pinned C++ compiler deliberately lowers repeat conditions through
        // the value pipeline, then emits one TEST/back-edge pair. This preserves
        // boolean materialization for comparison/and/or expressions instead of
        // reusing the generic condition jump-list pipeline.
        self.current_line = r.location.line;
        let cond = if matches!(r.condition.as_ref(), Expr::Nil(_)) {
            ValueResult::make_boolean(false)
        } else {
            self.emit_value(&r.condition)
        };
        self.check_codegen_error()?;
        let cond = self.force_single_value(cond);
        let cond_reg = self.value_to_any_reg(cond);

        self.remove_local_vars(block_level);
        self.current_line = r.condition.line();
        self.code_abc(OpCode::TEST, cond_reg, 0, 0, self.current_line);
        let repeat_jump = self.emit_jump();
        self.fix_jump(repeat_jump, loop_start);

        self.leave_block();
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // ForNum 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_for_num(&mut self, f: &ForNumStmt) -> Result<(), String> {
        self.current_line = f.location.line;
        let loop_line = f.location.line;
        let base = self.active_var_count;

        self.reg_alloc.set_freereg(base);

        // Lua numeric for uses four consecutive slots:
        // R(A)=internal index, R(A+1)=limit, R(A+2)=step, R(A+3)=visible variable.
        let init = self.emit_value(&f.init);
        self.check_codegen_error()?;
        let init = self.force_single_value(init);
        self.materialize_value(init, base);
        self.reg_alloc.set_freereg(base + 1);

        let mut limit = self.emit_value(&f.limit);
        self.check_codegen_error()?;
        limit = self.force_single_value(limit);
        self.materialize_value(limit, base + 1);
        self.reg_alloc.set_freereg(base + 2);

        let step = if let Some(ref s) = f.step {
            let sv = self.emit_value(s);
            self.force_single_value(sv)
        } else {
            ValueResult::make_number(1.0)
        };
        self.check_codegen_error()?;
        self.materialize_value(step, base + 2);
        self.reg_alloc.set_freereg(base + 3);

        self.enter_block(true);
        self.reg_alloc.set_freereg(base);
        self.add_local_var("(for index)");
        self.add_local_var("(for limit)");
        self.add_local_var("(for step)");
        self.add_local_var(f.var.clone());
        self.adjust_local_vars(4);

        // FORPREP
        let forprep_pc = self
            .builder
            .emit_as_bx(loop_line, OpCode::FORPREP, base, NO_JUMP);

        let loop_start = self.get_label();

        self.emit_block(&f.body)?;
        // The pinned C++ compiler emits an iteration-scope CLOSE for every
        // non-empty numeric-for body, independently of captured-local analysis.
        if !f.body.is_empty() {
            self.current_line = loop_line;
            self.emit_close(base + 3);
        }

        // FORLOOP
        let offset = loop_start - (self.get_label() + 1);
        let forloop_pc = self
            .builder
            .emit_as_bx(loop_line, OpCode::FORLOOP, base, offset);

        // FORPREP first jumps to FORLOOP; FORLOOP then enters the body if the
        // initial value is within range.
        self.fix_jump(forprep_pc, forloop_pc);
        self.leave_block();
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // ForIn 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_for_in(&mut self, f: &ForInStmt) -> Result<(), String> {
        self.current_line = f.location.line;
        let base = self.active_var_count;
        let nvars = f.vars.len() as i32;
        let nexps = f.iterators.len() as i32;
        let iterator_line = f
            .iterators
            .first()
            .map(|expr| expr.line())
            .unwrap_or(f.location.line);

        self.reg_alloc.set_freereg(base);

        let mut filled = 0;
        for (i, iter) in f.iterators.iter().enumerate() {
            if filled < 3 {
                let target_reg = base + filled;
                let wanted = 3 - filled;
                let is_last = i as i32 == nexps - 1;

                if is_last && let Expr::Call(call) = iter.as_ref() {
                    let mut info = self.emit_call_expr(call, target_reg);
                    self.check_codegen_error()?;
                    self.set_wanted_results(&mut info, wanted);
                    filled = 3;
                    self.reg_alloc.ensure_at_least(base + 3);
                    break;
                }

                if is_last && matches!(iter.as_ref(), Expr::Vararg(_)) {
                    let mut info = self.emit_vararg_expr();
                    self.set_multi_ret_base(&mut info, target_reg);
                    self.set_wanted_results(&mut info, wanted);
                    filled = 3;
                    self.reg_alloc.ensure_at_least(base + 3);
                    break;
                }

                let val = self.emit_value(iter);
                self.check_codegen_error()?;
                let val = self.force_single_value(val);
                self.materialize_value(val, target_reg);
                filled += 1;
                self.reg_alloc.ensure_at_least(base + filled);
            } else {
                let val = self.emit_value(iter);
                self.check_codegen_error()?;
                let val = self.force_single_value(val);
                let reg = self.value_to_any_reg(val);
                self.reg_alloc.free_reg(reg, self.active_var_count);
            }
        }

        while filled < 3 {
            self.materialize_value(ValueResult::make_nil(), base + filled);
            filled += 1;
        }

        self.enter_block(true);
        self.reg_alloc.set_freereg(base);
        self.add_local_var("(for generator)");
        self.add_local_var("(for state)");
        self.add_local_var("(for control)");
        for var_name in &f.vars {
            self.add_local_var(var_name.clone());
        }
        self.adjust_local_vars(3 + nvars);

        let jump_to_tfor = self.emit_jump();
        let loop_start = self.get_label();
        self.emit_block(&f.body)?;

        // The pinned C++ compiler closes the visible loop-variable scope on
        // every generic-for iteration, even when the body itself is empty. The
        // initial jump skips that CLOSE and lands directly on TFORLOOP.
        if nvars > 0 {
            self.current_line = iterator_line;
            self.emit_close(base + 3);
        }
        self.fix_jump(jump_to_tfor, self.get_label());
        self.builder
            .emit_abc(iterator_line, OpCode::TFORLOOP, base, 0, nvars);

        let offset = loop_start - (self.get_label() + 1);
        self.builder
            .emit_as_bx(iterator_line, OpCode::JMP, 0, offset);

        self.leave_block();
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // 函数定义语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_function_stmt(&mut self, f: &FunctionStmt) -> Result<(), String> {
        let def_line = f.location.line;
        self.current_line = def_line;

        if f.is_local {
            let reg = self.add_local_var(f.name.clone());
            self.emit_closure_to_reg(f, reg)?;
            self.adjust_local_vars(1);
        } else if f.table_path.is_empty() {
            let sym = self.resolve_name(&f.name);
            match sym.kind {
                SymbolKind::Local => {
                    let reg = self.reg_alloc.alloc();
                    self.emit_closure_to_reg(f, reg)?;
                    self.code_abc(OpCode::MOVE, sym.index, reg, 0, def_line);
                }
                SymbolKind::Upvalue => {
                    let reg = self.reg_alloc.alloc();
                    self.emit_closure_to_reg(f, reg)?;
                    self.code_abc(OpCode::SETUPVAL, reg, sym.index, 0, def_line);
                }
                SymbolKind::Global | SymbolKind::None => {
                    let reg = self.reg_alloc.alloc();
                    self.emit_closure_to_reg(f, reg)?;
                    let k = if sym.kind == SymbolKind::Global {
                        sym.index
                    } else {
                        self.add_string_constant(&f.name)
                            .unwrap_or_else(|| self.builder.add_number_constant(0.0))
                    };
                    self.code_abx(OpCode::SETGLOBAL, reg, k, def_line);
                }
            }
        } else {
            // 表成员函数: t.a.b.f() or t:m()
            let first_sym = self.resolve_name(&f.table_path[0]);
            let mut table_reg = if first_sym.kind == SymbolKind::Local {
                first_sym.index
            } else {
                let val = self.symbol_to_value(&first_sym);
                let reg = self.reg_alloc.alloc();
                self.materialize_value(val, reg);
                reg
            };

            for path_name in f.table_path.iter().skip(1) {
                let key = self
                    .add_string_constant(path_name)
                    .unwrap_or_else(|| self.builder.add_number_constant(0.0));
                let rk_key = if key <= MAXINDEXRK {
                    rk_ask(key)
                } else {
                    self.value_to_any_reg(ValueResult::make_constant(key))
                };
                let next_reg = self.reg_alloc.alloc();
                self.code_abc(OpCode::GETTABLE, next_reg, table_reg, rk_key, def_line);
                table_reg = next_reg;
            }

            let reg = self.reg_alloc.alloc();
            self.emit_closure_to_reg(f, reg)?;
            let key = self
                .add_string_constant(&f.name)
                .unwrap_or_else(|| self.builder.add_number_constant(0.0));
            let rk_key = if key <= MAXINDEXRK {
                rk_ask(key)
            } else {
                self.value_to_any_reg(ValueResult::make_constant(key))
            };
            self.code_abc(OpCode::SETTABLE, table_reg, rk_key, reg, def_line);
        }
        Ok(())
    }

    fn emit_closure_to_reg(&mut self, f: &FunctionStmt, reg: i32) -> Result<(), String> {
        let linedefined = f.location.line;
        let lastlinedefined = if f.end_line > 0 {
            f.end_line
        } else {
            linedefined
        };
        let function = self.compile_function(
            &f.params,
            f.is_vararg,
            &f.body,
            linedefined,
            lastlinedefined.max(linedefined),
        )?;
        self.code_abx(
            OpCode::CLOSURE,
            reg,
            function.proto_index,
            self.current_line,
        );
        self.emit_closure_upvalues(&function.upvalues);
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // Return 语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_return(&mut self, r: &ReturnStmt) {
        self.current_line = r.location.line;

        if r.values.is_empty() {
            self.code_abc(OpCode::RETURN, 0, 1, 0, self.current_line);
            return;
        }

        let nresults = r.values.len() as i32;
        let mut direct_local_base = None;
        for (index, value) in r.values.iter().enumerate() {
            let Expr::Name(name) = value.as_ref() else {
                direct_local_base = None;
                break;
            };
            let reg = self.find_local_var(&name.name);
            if reg < 0 {
                direct_local_base = None;
                break;
            }

            match direct_local_base {
                None if index == 0 => direct_local_base = Some(reg),
                Some(first) if reg == first + index as i32 => {}
                _ => {
                    direct_local_base = None;
                    break;
                }
            }
        }
        if let Some(base) = direct_local_base {
            self.code_abc(OpCode::RETURN, base, nresults + 1, 0, self.current_line);
            return;
        }

        // Return expressions use a fresh temporary range immediately above the
        // active locals. Condition lowering can leave higher scratch registers
        // live in the allocator, but the pinned C++ emitter discards that state
        // with RegisterGuard + setFreeReg(activeVarCount) before evaluating the
        // return list.
        let first_reg = self.active_var_count;
        self.reg_alloc.set_freereg(first_reg);

        if r.values.len() == 1
            && let Expr::Call(call) = r.values[0].as_ref()
        {
            let info = self.emit_call_expr(call, first_reg);
            if let Some(inst) = self.builder.instruction(info.instruction_pc) {
                let tail = crate::opcode::create_abc(
                    OpCode::TAILCALL,
                    crate::opcode::get_arg_a(inst),
                    crate::opcode::get_arg_b(inst),
                    0,
                );
                self.builder.replace_instruction(info.instruction_pc, tail);
            }
            self.code_abc(OpCode::RETURN, first_reg, 0, 0, self.current_line);
            return;
        }

        for (i, expr) in r
            .values
            .iter()
            .take(r.values.len().saturating_sub(1))
            .enumerate()
        {
            let val = self.emit_value(expr);
            let val = self.force_single_value(val);
            let target_reg = first_reg + i as i32;
            self.materialize_value(val, target_reg);
            self.reg_alloc.ensure_at_least(target_reg + 1);
        }

        let last_index = r.values.len() - 1;
        let last_reg = first_reg + last_index as i32;
        match r.values[last_index].as_ref() {
            Expr::Call(call) => {
                let mut info = self.emit_call_expr(call, last_reg);
                self.set_open_multi_ret(&mut info);
                self.code_abc(OpCode::RETURN, first_reg, 0, 0, self.current_line);
                return;
            }
            Expr::Vararg(_) => {
                let mut info = self.emit_vararg_expr();
                self.set_multi_ret_base(&mut info, last_reg);
                self.set_open_multi_ret(&mut info);
                self.code_abc(OpCode::RETURN, first_reg, 0, 0, self.current_line);
                return;
            }
            expr => {
                let val = self.emit_value(expr);
                let val = self.force_single_value(val);
                self.materialize_value(val, last_reg);
                self.reg_alloc.ensure_at_least(last_reg + 1);
            }
        }

        self.code_abc(
            OpCode::RETURN,
            first_reg,
            nresults + 1,
            0,
            self.current_line,
        );
    }

    // ═══════════════════════════════════════════════════════════════
    // Break 语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_break(&mut self) {
        let Some(level) = self
            .find_breakable_block()
            .map(|block| block.active_var_count)
        else {
            // Infallible enclosing emitters must still reject the whole Proto.
            // An unregistered NO_JUMP placeholder would execute as a self-jump.
            self.record_codegen_error("no loop to break");
            return;
        };
        self.close_scope_upvalues(level);
        let jump_pc = self.emit_jump();
        self.append_break_jump(jump_pc);
    }

    // ═══════════════════════════════════════════════════════════════
    // Do 块
    // ═══════════════════════════════════════════════════════════════

    fn emit_do(&mut self, d: &DoStmt) -> Result<(), String> {
        self.current_line = d.location.line;
        self.enter_block(false);
        self.emit_block(&d.body)?;
        self.current_line = d.location.line;
        self.leave_block();
        Ok(())
    }
}
