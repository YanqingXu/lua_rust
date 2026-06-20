//! 字节码生成器（CodeGenerator）
//!
//! 将 AST 转换为 Lua 5.1 字节码。
//!
//! ## 模块结构
//! - `types` — 核心数据流类型 (PatchList, ValueResult, CondResult, LValueRef, SymbolRef)
//! - `reg_alloc` — 临时寄存器分配/回收
//! - `builder` — 指令发射/常量写入/调试信息
//!
//! ## 待实现模块
//! - `expr_emit` — 表达式 lowering (对标 expression_emitter.hpp)
//! - `stmt_emit` — 语句/block/控制流 lowering (对标 statement_emitter.hpp)
//! - `binder` — 名字解析 (对标 name_binder.hpp)
//! - `jump` — 跳转链表管理 (对标 jump_patcher.hpp)
//! - `scope` — 局部变量/upvalue 作用域 (对标 scope_manager.hpp)
//! - `func_comp` — 子函数编译 + 闭包 (对标 function_compiler.hpp)
//!
//! C++ 参考: `lua_cpp/src/compiler/codegen/` (18 个文件)

pub mod builder;
pub mod reg_alloc;
pub mod types;
// pub mod binder;
// pub mod expr_emit;
// pub mod func_comp;
// pub mod jump;
// pub mod scope;
// pub mod stmt_emit;

// 常用类型重导出
pub use builder::BytecodeBuilder;
pub use reg_alloc::RegisterAllocator;
pub use types::{
    AccessKind, BlockInfo, CallResultInfo, CallResultKind, CompiledFunction, CondResult,
    LValueKind, LValueRef, LocalVar, NO_JUMP, PatchList, SymbolKind, SymbolRef, UpvalueCapture,
    ValueResult,
};

use lua_core::proto::Proto;

use crate::ast::stmt::{Chunk, Stmt};
use crate::opcode::OpCode;
use crate::parser::ParseError;

// =====================================================================
// CodegenError
// =====================================================================

/// 代码生成错误
///
/// C++ 对应: `Lua::CodegenError`
#[derive(Debug, Clone)]
pub struct CodegenError {
    pub message: String,
    pub line: i32,
    pub column: i32,
}

impl CodegenError {
    pub fn new(message: impl Into<String>, line: i32, column: i32) -> Self {
        Self {
            message: message.into(),
            line,
            column,
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for CodegenError {}

impl From<ParseError> for CodegenError {
    fn from(e: ParseError) -> Self {
        CodegenError::new(e.message, e.line, e.column)
    }
}

// =====================================================================
// CodeGenerator — 框架入口
// =====================================================================

/// Lua 5.1 字节码生成器
///
/// 将 AST 编译为可执行的 Proto 对象。
///
/// ## 当前状态
/// 框架已建立（类型系统、寄存器分配器、字节码构建器）。
/// 完整的表达式/语句 lowering 和函数编译待实现。
///
/// C++ 对应: `Lua::CodeGenerator`
pub struct CodeGenerator {
    /// 字节码构建器（拥有 Proto）
    pub builder: BytecodeBuilder,
    /// 寄存器分配器
    pub reg_alloc: RegisterAllocator,
    /// 当前行号
    pub current_line: i32,
}

impl CodeGenerator {
    /// 创建新的代码生成器
    pub fn new() -> Self {
        let proto = Proto::new();
        Self {
            builder: BytecodeBuilder::new(proto),
            reg_alloc: RegisterAllocator::new(0),
            current_line: 0,
        }
    }

    /// 生成字节码
    ///
    /// 当前为框架骨架：为 Chunk 中的每条语句预留位置。
    /// 完整实现需要 ExpressionEmitter + StatementEmitter + FunctionCompiler。
    ///
    /// C++ 对应: `Lua::CodeGenerator::generate()`
    pub fn generate(mut self, chunk: &Chunk, _source_name: &str) -> Result<Proto, CodegenError> {
        // 遍历顶层语句
        for stmt in &chunk.statements {
            self.current_line = stmt.line();
            self.emit_statement(stmt)?;
        }

        // 末尾兜底 RETURN
        let final_line = chunk.statements.last().map(|s| s.end_line()).unwrap_or(1);
        self.code_abc(OpCode::RETURN, 0, 1, 0, final_line);

        // 返回编译后的 Proto
        Ok(self.builder.into_proto())
    }

    // ── 指令生成便捷方法 ──────────────────────────────────────────

    fn code_abc(&mut self, op: OpCode, a: i32, b: i32, c: i32, line: i32) -> i32 {
        self.builder.emit_abc(line, op, a, b, c)
    }

    #[allow(dead_code)] // TODO: used by expression/statement emitters (pending)
    fn code_abx(&mut self, op: OpCode, a: i32, bx: i32, line: i32) -> i32 {
        self.builder.emit_abx(line, op, a, bx)
    }

    #[allow(dead_code)] // TODO: used by expression/statement emitters (pending)
    fn code_as_bx(&mut self, op: OpCode, a: i32, sbx: i32, line: i32) -> i32 {
        self.builder.emit_as_bx(line, op, a, sbx)
    }

    // ── 语句 lowering（骨架） ─────────────────────────────────────

    fn emit_statement(&mut self, stmt: &Stmt) -> Result<(), CodegenError> {
        match stmt {
            Stmt::Empty(_) => {}
            Stmt::Assign(a) => {
                let _ = a;
            }
            Stmt::Local(l) => {
                let _ = l;
            }
            Stmt::Call(c) => {
                let _ = c;
            }
            Stmt::If(i) => {
                let _ = i;
            }
            Stmt::While(w) => {
                let _ = w;
            }
            Stmt::Repeat(r) => {
                let _ = r;
            }
            Stmt::ForNum(f) => {
                let _ = f;
            }
            Stmt::ForIn(f) => {
                let _ = f;
            }
            Stmt::Function(f) => {
                let _ = f;
            }
            Stmt::Return(r) => {
                let _ = r;
            }
            Stmt::Break(_) => {}
            Stmt::Do(d) => {
                let _ = d;
            }
        }
        Ok(())
    }
}

impl Default for CodeGenerator {
    fn default() -> Self {
        Self::new()
    }
}
