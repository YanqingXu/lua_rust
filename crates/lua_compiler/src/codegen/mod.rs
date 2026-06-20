//! 字节码生成器（CodeGenerator）
//!
//! 将 AST 转换为 Lua 5.1 字节码。
//!
//! ## 模块结构
//! - `types` — 核心数据流类型 (PatchList, ValueResult, CondResult, LValueRef, SymbolRef)
//! - `reg_alloc` — 临时寄存器分配/回收
//! - `builder` — 指令发射/常量写入/调试信息
//! - `jump` — 跳转链表管理与回填 (jump_patcher.hpp)
//! - `scope` — 局部变量/upvalue/block 作用域 (scope_manager.hpp)
//! - `binder` — 名字解析 SymbolRef (name_binder.hpp)
//! - `expr_emit` — 表达式 lowering (expression_emitter.hpp)
//! - `stmt_emit` — 语句/block/控制流 lowering (statement_emitter.hpp)
//! - `func_comp` — 子函数编译与闭包 (function_compiler.hpp)
//!
//! C++ 参考: `lua_cpp/src/compiler/codegen/` (18 个文件)

pub mod binder;
pub mod builder;
pub mod expr_emit;
pub mod func_comp;
pub mod jump;
pub mod reg_alloc;
pub mod scope;
pub mod stmt_emit;
pub mod types;

pub use builder::BytecodeBuilder;
pub use reg_alloc::RegisterAllocator;
pub use types::{
    AccessKind, BlockInfo, CallResultInfo, CallResultKind, CompiledFunction, CondResult,
    LValueKind, LValueRef, LocalVar, NO_JUMP, PatchList, SymbolKind, SymbolRef, UpvalueCapture,
    ValueResult,
};

use lua_core::proto::Proto;

use crate::ast::stmt::Chunk;
use crate::opcode::OpCode;
use crate::parser::ParseError;

// =====================================================================
// CodegenError
// =====================================================================

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
// CodeGenerator
// =====================================================================

/// Lua 5.1 字节码生成器
///
/// 将 AST 编译为可执行的 Proto 对象。
/// 实现完整的表达式/语句 lowering 和函数编译管线。
///
/// C++ 对应: `Lua::CodeGenerator`
pub struct CodeGenerator {
    pub builder: BytecodeBuilder,
    pub reg_alloc: RegisterAllocator,
    pub current_line: i32,

    // ── 跳转管理 ──────────────────────────────────────────────
    pub jpc: i32,

    // ── 局部变量 ──────────────────────────────────────────────
    pub local_vars: Vec<LocalVar>,
    pub active_var_count: i32,

    // ── Upvalue ───────────────────────────────────────────────
    pub upvalues: Vec<UpvalueCapture>,

    // ── 代码块栈 ──────────────────────────────────────────────
    pub blocks: Vec<BlockInfo>,
}

impl CodeGenerator {
    pub fn new() -> Self {
        let proto = Proto::new();
        Self {
            builder: BytecodeBuilder::new(proto),
            reg_alloc: RegisterAllocator::new(0),
            current_line: 0,
            jpc: NO_JUMP,
            local_vars: Vec::new(),
            active_var_count: 0,
            upvalues: Vec::new(),
            blocks: Vec::new(),
        }
    }

    /// 生成字节码（完整入口）
    pub fn generate(mut self, chunk: &Chunk, _source_name: &str) -> Result<Proto, CodegenError> {
        self.builder.set_max_stack_size(2);
        self.builder.set_vararg(true);

        self.emit_block(&chunk.statements)
            .map_err(|msg| CodegenError::new(msg, 0, 0))?;

        let final_line = chunk.statements.last().map(|s| s.end_line()).unwrap_or(1);
        self.code_abc(OpCode::RETURN, 0, 1, 0, final_line);

        Ok(self.builder.into_proto())
    }

    // ── 指令生成便捷方法 ──────────────────────────────────────────

    pub fn code_abc(&mut self, op: OpCode, a: i32, b: i32, c: i32, line: i32) -> i32 {
        self.builder.emit_abc(line, op, a, b, c)
    }

    pub fn code_abx(&mut self, op: OpCode, a: i32, bx: i32, line: i32) -> i32 {
        self.builder.emit_abx(line, op, a, bx)
    }

    pub fn code_as_bx(&mut self, op: OpCode, a: i32, sbx: i32, line: i32) -> i32 {
        self.builder.emit_as_bx(line, op, a, sbx)
    }
}

impl Default for CodeGenerator {
    fn default() -> Self {
        Self::new()
    }
}
