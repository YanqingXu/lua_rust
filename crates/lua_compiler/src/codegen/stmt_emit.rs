//! 语句发射器 (Statement Emitter)
//!
//! 实现 AST 语句 → 字节码的 lowering：赋值、局部声明、控制流、
//! 循环、return、函数定义等全部 13 种 Stmt 节点。
//!
//! C++ 参考: `lua_cpp/src/compiler/codegen/statement_emitter.hpp/.cpp`

use crate::ast::expr::Expr;
use crate::ast::stmt::{
    AssignStmt, CallStmt, DoStmt, ForInStmt, ForNumStmt, FunctionStmt, IfStmt, LocalStmt,
    RepeatStmt, ReturnStmt, Stmt, WhileStmt,
};
use crate::codegen::CodeGenerator;
use crate::codegen::types::{NO_JUMP, ValueResult};
use crate::opcode::OpCode;

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
            Stmt::Break(_) => self.emit_break(),
            Stmt::Do(d) => self.emit_do(d),
        }
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

        // 求值所有右值，强制为单值
        let mut values: Vec<ValueResult> = Vec::new();
        for v in &a.values {
            let val = self.emit_value(v);
            values.push(self.force_single_value(val));
        }

        // 存储到左值目标
        for (i, target_expr) in a.targets.iter().enumerate() {
            let lv = self.emit_lvalue(target_expr)?;
            let val = if i < values.len() {
                values[i].clone()
            } else {
                ValueResult::make_nil()
            };
            self.emit_store(&lv, val);
        }
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    // 局部变量声明
    // ═══════════════════════════════════════════════════════════════

    fn emit_local(&mut self, l: &LocalStmt) {
        self.current_line = l.location.line;
        let nvars = l.names.len() as i32;

        // 先求值所有初始值
        let mut values: Vec<ValueResult> = Vec::new();
        for v in &l.values {
            let val = self.emit_value(v);
            values.push(self.force_single_value(val));
        }

        // 为每个局部变量分配寄存器
        for (i, name) in l.names.iter().enumerate() {
            self.add_local_var(name.clone());
            if i < values.len() {
                self.materialize_value(values[i].clone(), self.reg_alloc.current() - 1);
            }
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

            let _ = self.emit_block(&branch.body);

            if idx < i.branches.len() - 1 || !i.else_branch.is_empty() {
                exit_list.push(self.emit_jump());
            }
            self.patch_list_to_here(&cond.false_list);
        }

        if !i.else_branch.is_empty() {
            let _ = self.emit_block(&i.else_branch);
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
        self.enter_block(true);

        let _loop_start = self.get_label();
        let _ = self.emit_block(&r.body);

        let cond = self.emit_cond_result(&r.condition);
        self.patch_list_to_here(&cond.false_list); // false → 重复
        self.patch_list_to_here(&cond.true_list); // true → 退出

        self.leave_block();
    }

    // ═══════════════════════════════════════════════════════════════
    // ForNum 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_for_num(&mut self, f: &ForNumStmt) {
        self.current_line = f.location.line;

        // 分配循环变量寄存器
        let var_reg = self.add_local_var(f.var.clone());
        self.adjust_local_vars(1);

        // init, limit, step → 暂存
        let init = self.emit_value(&f.init);
        let init = self.force_single_value(init);
        let mut limit = self.emit_value(&f.limit);
        limit = self.force_single_value(limit);
        let step = if let Some(ref s) = f.step {
            let sv = self.emit_value(s);
            self.force_single_value(sv)
        } else {
            ValueResult::make_number(1.0)
        };

        // 物化到寄存器: i, limit, step
        self.materialize_value(init, var_reg);
        let limit_reg = self.reg_alloc.alloc();
        self.materialize_value(limit, limit_reg);
        let step_reg = self.reg_alloc.alloc();
        self.materialize_value(step, step_reg);

        // FORPREP
        let forprep_pc =
            self.builder
                .emit_as_bx(self.current_line, OpCode::FORPREP, var_reg, NO_JUMP);

        let loop_start = self.get_label();
        self.enter_block(true);

        let _ = self.emit_block(&f.body);

        self.leave_block();

        // FORLOOP
        let offset = loop_start - (self.get_label() + 1);
        self.builder
            .emit_as_bx(self.current_line, OpCode::FORLOOP, var_reg, offset);

        // 回填 FORPREP
        let forprep_offset = self.get_label() - (forprep_pc + 1);
        let mut forprep_inst = self.builder.instruction(forprep_pc).unwrap();
        crate::opcode::set_arg_sbx(&mut forprep_inst, forprep_offset);
        self.builder.replace_instruction(forprep_pc, forprep_inst);
    }

    // ═══════════════════════════════════════════════════════════════
    // ForIn 循环
    // ═══════════════════════════════════════════════════════════════

    fn emit_for_in(&mut self, f: &ForInStmt) {
        self.current_line = f.location.line;

        for var_name in &f.vars {
            self.add_local_var(var_name.clone());
        }
        self.adjust_local_vars(f.vars.len() as i32);

        // 求值迭代器表达式
        for iter in &f.iterators {
            let val = self.emit_value(iter);
            let val = self.force_single_value(val);
            let reg = self.value_to_any_reg(val);
            self.reg_alloc.ensure_at_least(reg + 1);
        }

        // TFORLOOP
        let base = self.reg_alloc.current() - f.iterators.len() as i32;
        let _tfor_pc = self.builder.emit_abc(
            self.current_line,
            OpCode::TFORLOOP,
            base,
            0,
            f.vars.len() as i32 + 1,
        );

        self.enter_block(true);
        let _ = self.emit_block(&f.body);
        self.leave_block();
    }

    // ═══════════════════════════════════════════════════════════════
    // 函数定义语句
    // ═══════════════════════════════════════════════════════════════

    fn emit_function_stmt(&mut self, f: &FunctionStmt) -> Result<(), String> {
        self.current_line = f.location.line;

        if f.is_local {
            // local function — 寄存器已在 parse_local_stmt 中分配
            self.emit_closure_to_reg(f, self.reg_alloc.current() - 1)?;
        } else if f.table_path.is_empty() {
            // 简单全局函数
            let reg = self.reg_alloc.alloc();
            self.emit_closure_to_reg(f, reg)?;
            // SETGLOBAL
            let k = self.builder.add_number_constant(0.0); // placeholder
            self.code_abx(OpCode::SETGLOBAL, reg, k, self.current_line);
        } else {
            // 表成员函数: t.a.b.f() or t:m()
            let reg = self.reg_alloc.alloc();
            self.emit_closure_to_reg(f, reg)?;
            // TODO: SETTABLE through table path
        }
        Ok(())
    }

    fn emit_closure_to_reg(&mut self, f: &FunctionStmt, reg: i32) -> Result<(), String> {
        let _linedefined = f.location.line;
        let _lastlinedefined = f.end_line;
        // TODO: actually compile sub-function
        self.code_abx(OpCode::CLOSURE, reg, 0, self.current_line);
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

        // 求值返回值并物化到连续寄存器
        let first_reg = self.reg_alloc.current();
        for (i, expr) in r.values.iter().enumerate() {
            let val = self.emit_value(expr);
            let val = self.force_single_value(val);
            let target_reg = first_reg + i as i32;
            self.materialize_value(val, target_reg);
            self.reg_alloc.ensure_at_least(target_reg + 1);
        }

        let nresults = r.values.len() as i32;
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
        let _ = self.emit_block(&d.body);
    }
}
