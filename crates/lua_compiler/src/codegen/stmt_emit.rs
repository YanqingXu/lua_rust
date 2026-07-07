//! 语句发射器 (Statement Emitter)
//!
//! 实现 AST 语句 → 字节码的 lowering：赋值、局部声明、控制流、
//! 循环、return、函数定义等全部 13 种 Stmt 节点。
//!

use crate::ast::expr::Expr;
use crate::ast::stmt::{
    AssignStmt, CallStmt, DoStmt, ForInStmt, ForNumStmt, FunctionStmt, IfStmt, LocalStmt,
    RepeatStmt, ReturnStmt, Stmt, WhileStmt,
};
use crate::codegen::CodeGenerator;
use crate::codegen::types::{AccessKind, LValueKind, NO_JUMP, SymbolKind, ValueResult};
use crate::opcode::{MAXINDEXRK, OpCode, is_k, rk_ask};

impl CodeGenerator {
    // ═══════════════════════════════════════════════════════════════
    // 公开入口 — 语句 / 语句块
    // ═══════════════════════════════════════════════════════════════

    /// 发射单条语句
    pub fn emit_statement(&mut self, stmt: &Stmt) -> Result<(), String> {
        match stmt {
            Stmt::Empty(_) => {}
            Stmt::Assign(a) => self.emit_assign(a)?,
            Stmt::Local(l) => self.emit_local(l),
            Stmt::Call(c) => self.emit_call_stmt(c)?,
            Stmt::If(i) => self.emit_if(i),
            Stmt::While(w) => self.emit_while(w),
            Stmt::Repeat(r) => self.emit_repeat(r),
            Stmt::ForNum(f) => self.emit_for_num(f),
            Stmt::ForIn(f) => self.emit_for_in(f),
            Stmt::Function(f) => self.emit_function_stmt(f)?,
            Stmt::Return(r) => self.emit_return(r),
            Stmt::Break(b) => {
                self.current_line = b.location.line;
                self.emit_break();
            }
            Stmt::Do(d) => self.emit_do(d),
        }
        self.reg_alloc.reset_to_locals(self.active_var_count);
        Ok(())
    }

    /// 发射语句块
    pub fn emit_block(&mut self, stmts: &[Box<Stmt>]) -> Result<(), String> {
        for stmt in stmts {
            self.emit_statement(stmt)?;
        }
        Ok(())
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

        for target in &a.targets {
            if let Expr::Name(name) = target.as_ref() {
                let _ = self.resolve_name(&name.name);
            }
        }

        let value_base = self.reg_alloc.current();
        let mut assigned_values = 0;
        let direct_values = nvars.min(nexps);

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

            let val = self.emit_value(expr);
            let val = self.force_single_value(val);
            self.materialize_value(val, target_reg);
            self.reg_alloc.ensure_at_least(target_reg + 1);
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
        }

        self.reg_alloc.set_freereg(value_base + nvars);
        let mut targets = Vec::with_capacity(a.targets.len());
        for target_expr in &a.targets {
            let mut lv = self.emit_lvalue(target_expr)?;
            if lv.kind == LValueKind::Indexed {
                let table_tmp = self.reg_alloc.alloc();
                self.code_abc(OpCode::MOVE, table_tmp, lv.table_reg, 0, self.current_line);
                lv.table_reg = table_tmp;

                if !is_k(lv.key) {
                    let key_tmp = self.reg_alloc.alloc();
                    self.code_abc(OpCode::MOVE, key_tmp, lv.key, 0, self.current_line);
                    lv.key = key_tmp;
                }
            }
            targets.push(lv);
        }

        for (i, lv) in targets.iter().enumerate() {
            let val = if (i as i32) < assigned_values {
                ValueResult::make_register(value_base + i as i32, false, AccessKind::None)
            } else {
                ValueResult::make_nil()
            };
            self.emit_store(lv, val);
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

        let mut initialized = 0;
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
                let mut info = self.emit_call_expr(call, target_reg);
                self.set_wanted_results(&mut info, wanted);
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

        if initialized < nvars {
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
            let _info = self.emit_call_expr(call, -1);
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // If 语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_if(&mut self, i: &IfStmt) {
        self.current_line = i.location.line;
        let mut exit_list: Vec<i32> = Vec::new();

        for (idx, branch) in i.branches.iter().enumerate() {
            self.current_line = branch.condition.line();
            let cond = self.emit_cond_result(&branch.condition);
            self.patch_list_to_here(&cond.true_list);

            self.enter_block(false);
            let _ = self.emit_block(&branch.body);
            self.leave_block();

            if idx < i.branches.len() - 1 || !i.else_branch.is_empty() {
                exit_list.push(self.emit_jump());
            }
            self.patch_list_to_here(&cond.false_list);
        }

        if !i.else_branch.is_empty() {
            self.enter_block(false);
            let _ = self.emit_block(&i.else_branch);
            self.leave_block();
        }

        let target = self.get_label();
        for pc in exit_list {
            self.fix_jump(pc, target);
        }
    }

    // ═══════════════════════════════════════════════════════════════
    // While 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_while(&mut self, w: &WhileStmt) {
        self.current_line = w.location.line;
        self.enter_block(true);

        let loop_start = self.get_label();
        let cond = self.emit_cond_result(&w.condition);
        self.patch_list_to_here(&cond.true_list);

        let _ = self.emit_block(&w.body);

        // 跳回循环头
        let offset = loop_start - (self.get_label() + 1);
        self.builder
            .emit_as_bx(self.current_line, OpCode::JMP, 0, offset);

        self.patch_list_to_here(&cond.false_list);
        self.leave_block();
    }

    // ═══════════════════════════════════════════════════════════════
    // Repeat 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_repeat(&mut self, r: &RepeatStmt) {
        self.current_line = r.location.line;
        let block_level = self.active_var_count;
        self.enter_block(true);

        let loop_start = self.get_label();
        let _ = self.emit_block(&r.body);

        self.current_line = r.condition.line();
        let cond = self.emit_cond_result(&r.condition);
        let exit_jump = self.emit_jump();
        let repeat_false_target = self.get_label();
        let _ = self.close_scope_upvalues(block_level);
        let offset = loop_start - (self.get_label() + 1);
        self.builder
            .emit_as_bx(self.current_line, OpCode::JMP, 0, offset);
        self.patch_list_vec(&cond.false_list, repeat_false_target); // false → 重复
        self.fix_jump(exit_jump, self.get_label());
        self.patch_list_to_here(&cond.true_list); // true → 退出

        self.leave_block();
    }

    // ═══════════════════════════════════════════════════════════════
    // ForNum 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_for_num(&mut self, f: &ForNumStmt) {
        self.current_line = f.location.line;
        let loop_line = f.location.line;
        let base = self.active_var_count;

        self.reg_alloc.set_freereg(base);

        // Lua numeric for uses four consecutive slots:
        // R(A)=internal index, R(A+1)=limit, R(A+2)=step, R(A+3)=visible variable.
        let init = self.emit_value(&f.init);
        let init = self.force_single_value(init);
        self.materialize_value(init, base);
        self.reg_alloc.set_freereg(base + 1);

        let mut limit = self.emit_value(&f.limit);
        limit = self.force_single_value(limit);
        self.materialize_value(limit, base + 1);
        self.reg_alloc.set_freereg(base + 2);

        let step = if let Some(ref s) = f.step {
            let sv = self.emit_value(s);
            self.force_single_value(sv)
        } else {
            ValueResult::make_number(1.0)
        };
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

        let _ = self.emit_block(&f.body);
        let _ = self.close_scope_upvalues(base + 3);

        // FORLOOP
        let offset = loop_start - (self.get_label() + 1);
        let forloop_pc = self
            .builder
            .emit_as_bx(loop_line, OpCode::FORLOOP, base, offset);

        // FORPREP first jumps to FORLOOP; FORLOOP then enters the body if the
        // initial value is within range.
        self.fix_jump(forprep_pc, forloop_pc);
        self.leave_block();
    }

    // ═══════════════════════════════════════════════════════════════
    // ForIn 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_for_in(&mut self, f: &ForInStmt) {
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
                let val = self.force_single_value(val);
                self.materialize_value(val, target_reg);
                filled += 1;
                self.reg_alloc.ensure_at_least(base + filled);
            } else {
                let val = self.emit_value(iter);
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
        let _ = self.emit_block(&f.body);
        let _ = self.close_scope_upvalues(base + 3);

        self.fix_jump(jump_to_tfor, self.get_label());
        self.builder
            .emit_abc(iterator_line, OpCode::TFORLOOP, base, 0, nvars);

        let offset = loop_start - (self.get_label() + 1);
        self.builder
            .emit_as_bx(iterator_line, OpCode::JMP, 0, offset);

        self.leave_block();
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
                    self.emit_closure_to_reg(f, sym.index)?;
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
                        // SAFETY: self.gc is set from a valid &mut GC during CodeGenerator::new()
                        let gc: &mut lua_core::gc::collector::GarbageCollector =
                            unsafe { &mut *self.gc };
                        self.builder
                            .add_string_constant(gc, &f.name)
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
                // SAFETY: self.gc is set from a valid &mut GC during CodeGenerator::new()
                let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
                let key = self
                    .builder
                    .add_string_constant(gc, path_name)
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
            // SAFETY: self.gc is set from a valid &mut GC during CodeGenerator::new()
            let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
            let key = self
                .builder
                .add_string_constant(gc, &f.name)
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

        let first_reg = self.reg_alloc.current();
        let nresults = r.values.len() as i32;

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
        let jump_pc = self.emit_jump();
        self.append_break_jump(jump_pc);
    }

    // ═══════════════════════════════════════════════════════════════
    // Do 块
    // ═══════════════════════════════════════════════════════════════

    fn emit_do(&mut self, d: &DoStmt) {
        self.current_line = d.location.line;
        self.enter_block(false);
        let _ = self.emit_block(&d.body);
        self.leave_block();
    }
}
