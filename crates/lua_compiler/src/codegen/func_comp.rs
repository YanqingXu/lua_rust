//! 函数编译器 (Function Compiler)
//!
//! 管理子函数原型的编译生命周期：参数绑定、函数体编译、
//! upvalue 闭包元数据和调试信息附加。
//!
//! C++ 参考: `lua_cpp/src/compiler/codegen/function_compiler.hpp/.cpp`

use lua_core::gc_string::GcString;
use lua_core::proto::{Proto, VARARG_ISVARARG, VARARG_NEEDSARG};

use crate::ast::expr::Expr;
use crate::ast::stmt::Stmt;
use crate::codegen::CodeGenerator;
use crate::codegen::types::{CompiledFunction, ParentFunctionContext, UpvalueCapture};
use crate::opcode::OpCode;

impl CodeGenerator {
    // ═══════════════════════════════════════════════════════════════
    // 函数编译
    // ═══════════════════════════════════════════════════════════════

    /// 编译子函数体，返回 CompiledFunction
    ///
    /// C++ 对应: `FunctionCompiler::compile()`
    pub fn compile_function(
        &mut self,
        params: &[String],
        is_vararg: bool,
        body: &[Box<crate::ast::stmt::Stmt>],
        linedefined: i32,
        _lastlinedefined: i32,
    ) -> Result<CompiledFunction, String> {
        let vararg_usage = scan_vararg_usage(body);
        let needs_arg = is_vararg && !vararg_usage.uses_vararg_expr;

        // 创建子 Proto
        let source = self.builder.proto().source();
        let mut sub_proto = Proto::new();
        let vararg_flags = if is_vararg {
            VARARG_ISVARARG | if needs_arg { VARARG_NEEDSARG } else { 0 }
        } else {
            0
        };
        sub_proto.set_source(source);
        sub_proto.set_vararg_flags(vararg_flags);
        sub_proto.set_line_defined(linedefined);
        sub_proto.set_last_line_defined(_lastlinedefined);

        // 保存当前状态
        let saved_builder = std::mem::replace(
            &mut self.builder,
            crate::codegen::builder::BytecodeBuilder::new(sub_proto),
        );
        let saved_reg_alloc = std::mem::replace(
            &mut self.reg_alloc,
            crate::codegen::RegisterAllocator::new(0),
        );
        let parent_local_vars = std::mem::take(&mut self.local_vars);
        let saved_active_vars = self.active_var_count;
        self.active_var_count = 0;
        let parent_upvalues = std::mem::take(&mut self.upvalues);
        self.parent_functions.push(ParentFunctionContext::new(
            parent_local_vars,
            parent_upvalues,
        ));
        let saved_blocks = std::mem::take(&mut self.blocks);
        let saved_jpc = self.jpc;
        self.jpc = crate::codegen::types::NO_JUMP;

        // 设置参数数量
        let num_params = params.len();
        self.builder.set_num_params(num_params as u8);

        // 为参数分配寄存器
        for param in params.iter() {
            if param == "..." {
                continue; // vararg marker handled separately
            }
            self.add_local_var(param.clone());
        }
        self.adjust_local_vars(params.len() as i32);
        if needs_arg {
            self.add_local_var("arg");
            self.adjust_local_vars(1);
        }

        // 编译函数体
        self.emit_block(body)?;

        // 末尾兜底 RETURN
        self.code_abc(OpCode::RETURN, 0, 1, 0, self.current_line);

        // 附加调试信息
        self.attach_local_debug();

        // 保存编译结果
        let upvalues = self.upvalues.clone();
        self.builder.set_num_upvalues(upvalues.len() as u8);
        // SAFETY: self.gc is set during CodeGenerator::new() from a valid &mut
        // GarbageCollector reference that outlives the compilation process.
        let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
        for upvalue in &upvalues {
            let name = gc.create(GcString::new(&upvalue.name));
            self.builder.add_upvalue_name(name);
        }
        let max_stack = (self.reg_alloc.max_used() + 2).max(self.active_var_count + 2);
        self.builder.set_max_stack_size(max_stack as u8);

        let child_builder = std::mem::replace(&mut self.builder, saved_builder);
        let child_proto = child_builder.into_proto();

        let child_ref = gc.create(child_proto);
        let proto_index = self.builder.add_sub_proto(child_ref);

        // 恢复状态
        let parent_context = self
            .parent_functions
            .pop()
            .expect("parent function context should exist");
        self.reg_alloc = saved_reg_alloc;
        self.local_vars = parent_context.local_vars;
        self.active_var_count = saved_active_vars;
        self.upvalues = parent_context.upvalues;
        self.blocks = saved_blocks;
        self.jpc = saved_jpc;

        Ok(CompiledFunction {
            proto_index,
            upvalues,
        })
    }

    /// 发射闭包 upvalue 指令（在 CLOSURE 之后）
    ///
    /// C++ 对应: `FunctionCompiler::emitClosureUpvalues()`
    pub fn emit_closure_upvalues(&mut self, upvalues: &[UpvalueCapture]) {
        for uv in upvalues {
            if uv.in_stack {
                self.code_abc(OpCode::MOVE, 0, uv.index, 0, self.current_line);
            } else {
                self.code_abc(OpCode::GETUPVAL, 0, uv.index, 0, self.current_line);
            }
        }
    }
}

#[derive(Default)]
struct VarargUsage {
    uses_arg_name: bool,
    uses_vararg_expr: bool,
}

fn scan_vararg_usage(body: &[Box<Stmt>]) -> VarargUsage {
    let mut usage = VarargUsage::default();
    for stmt in body {
        scan_stmt(stmt, &mut usage);
    }
    usage
}

fn scan_stmt(stmt: &Stmt, usage: &mut VarargUsage) {
    match stmt {
        Stmt::Empty(_) | Stmt::Break(_) => {}
        Stmt::Assign(stmt) => {
            scan_exprs(&stmt.targets, usage);
            scan_exprs(&stmt.values, usage);
        }
        Stmt::Local(stmt) => scan_exprs(&stmt.values, usage),
        Stmt::Call(stmt) => scan_expr(&stmt.call, usage),
        Stmt::If(stmt) => {
            for branch in &stmt.branches {
                scan_expr(&branch.condition, usage);
                scan_stmts(&branch.body, usage);
            }
            scan_stmts(&stmt.else_branch, usage);
        }
        Stmt::While(stmt) => {
            scan_expr(&stmt.condition, usage);
            scan_stmts(&stmt.body, usage);
        }
        Stmt::Repeat(stmt) => {
            scan_stmts(&stmt.body, usage);
            scan_expr(&stmt.condition, usage);
        }
        Stmt::ForNum(stmt) => {
            scan_expr(&stmt.init, usage);
            scan_expr(&stmt.limit, usage);
            if let Some(step) = &stmt.step {
                scan_expr(step, usage);
            }
            scan_stmts(&stmt.body, usage);
        }
        Stmt::ForIn(stmt) => {
            scan_exprs(&stmt.iterators, usage);
            scan_stmts(&stmt.body, usage);
        }
        Stmt::Function(_) => {}
        Stmt::Return(stmt) => scan_exprs(&stmt.values, usage),
        Stmt::Do(stmt) => scan_stmts(&stmt.body, usage),
    }
}

fn scan_stmts(stmts: &[Box<Stmt>], usage: &mut VarargUsage) {
    for stmt in stmts {
        scan_stmt(stmt, usage);
    }
}

fn scan_exprs(exprs: &[Box<Expr>], usage: &mut VarargUsage) {
    for expr in exprs {
        scan_expr(expr, usage);
    }
}

fn scan_expr(expr: &Expr, usage: &mut VarargUsage) {
    match expr {
        Expr::Nil(_) | Expr::Boolean(_) | Expr::Number(_) | Expr::String(_) => {}
        Expr::Vararg(_) => usage.uses_vararg_expr = true,
        Expr::Name(expr) => {
            if expr.name == "arg" {
                usage.uses_arg_name = true;
            }
        }
        Expr::Binary(expr) => {
            scan_expr(&expr.left, usage);
            scan_expr(&expr.right, usage);
        }
        Expr::Unary(expr) => scan_expr(&expr.operand, usage),
        Expr::Table(expr) => {
            for field in &expr.fields {
                if let Some(key) = &field.key {
                    scan_expr(key, usage);
                }
                scan_expr(&field.value, usage);
            }
        }
        Expr::Call(expr) => {
            scan_expr(&expr.func, usage);
            scan_exprs(&expr.args, usage);
        }
        Expr::Index(expr) => {
            scan_expr(&expr.table, usage);
            scan_expr(&expr.index, usage);
        }
        Expr::Member(expr) => scan_expr(&expr.table, usage),
        Expr::Function(_) => {}
        Expr::Paren(expr) => scan_expr(&expr.expression, usage),
    }
}
