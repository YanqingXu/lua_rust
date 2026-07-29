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
    LValueKind, LValueRef, LocalVar, NO_JUMP, ParentFunctionContext, PatchList, SymbolKind,
    SymbolRef, UpvalueCapture, ValueResult,
};

use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc::publication::PublicationTxn;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;

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

mod allocator_sealed {
    pub trait Sealed {}

    impl Sealed for lua_core::gc::collector::GarbageCollector {}
    impl Sealed for lua_core::gc::publication::PublicationTxn<'_> {}
}

/// Allocation substrate used by the compiler's Proto builder.
///
/// The trait is sealed so production callers select either direct test
/// allocation or a lexical publication transaction. String allocation always
/// requires the canonical pool.
#[doc(hidden)]
pub trait CodegenObjectAllocator: allocator_sealed::Sealed {
    fn allocate_string(&mut self, string_pool: &mut StringPool, bytes: &[u8]) -> GcRef<GcString>;

    fn allocate_proto(&mut self, proto: Proto) -> GcRef<Proto>;
}

impl CodegenObjectAllocator for GarbageCollector {
    fn allocate_string(&mut self, string_pool: &mut StringPool, bytes: &[u8]) -> GcRef<GcString> {
        string_pool.intern_bytes(self, bytes)
    }

    fn allocate_proto(&mut self, proto: Proto) -> GcRef<Proto> {
        self.create(proto)
    }
}

impl CodegenObjectAllocator for PublicationTxn<'_> {
    fn allocate_string(&mut self, string_pool: &mut StringPool, bytes: &[u8]) -> GcRef<GcString> {
        let rooted = self
            .intern_bytes(string_pool, bytes)
            .expect("compiler StringPool entry belongs to the publication collector");
        // SAFETY: CodeGenerator attaches this handle only to the unpublished
        // Proto tree owned by the same transaction.
        unsafe { self.retain_string_for_proto(rooted) }
            .expect("new compiler string remains protected")
    }

    fn allocate_proto(&mut self, proto: Proto) -> GcRef<Proto> {
        let rooted = self.alloc(proto);
        // SAFETY: CodeGenerator immediately attaches this child to its
        // unpublished parent, and the same transaction protects both.
        unsafe { self.retain_proto_for_parent(rooted) }
            .expect("new compiler Proto remains protected")
    }
}

/// Lua 5.1 字节码生成器
///
/// 将 AST 编译为可执行的 Proto 对象。
/// 实现完整的表达式/语句 lowering 和函数编译管线。
pub struct CodeGenerator<'services, A: CodegenObjectAllocator + ?Sized = GarbageCollector> {
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
    pub parent_functions: Vec<ParentFunctionContext>,

    // ── 代码块栈 ──────────────────────────────────────────────
    pub blocks: Vec<BlockInfo>,

    /// 编译期间借用的对象分配服务；生命周期保证服务不会先于编译器失效。
    allocator: &'services mut A,

    /// Canonical string pool, borrowed for the complete compilation pass.
    string_pool: &'services mut StringPool,
}

impl<'services> CodeGenerator<'services, GarbageCollector> {
    pub fn new_with_pool(
        gc: &'services mut GarbageCollector,
        string_pool: &'services mut StringPool,
    ) -> Self {
        Self::from_services(gc, string_pool)
    }
}

impl<'services, 'scope> CodeGenerator<'services, PublicationTxn<'scope>> {
    /// Create a publication-rooted compiler with canonical string interning.
    pub fn new_in_publication_with_pool(
        transaction: &'services mut PublicationTxn<'scope>,
        string_pool: &'services mut StringPool,
    ) -> Self {
        Self::from_services(transaction, string_pool)
    }
}

impl<'services, A: CodegenObjectAllocator + ?Sized> CodeGenerator<'services, A> {
    fn from_services(allocator: &'services mut A, string_pool: &'services mut StringPool) -> Self {
        let mut proto = Proto::new();
        proto.set_max_stack_size(2);
        Self {
            builder: BytecodeBuilder::new(proto),
            reg_alloc: RegisterAllocator::new(0),
            current_line: 0,
            jpc: NO_JUMP,
            local_vars: Vec::new(),
            active_var_count: 0,
            upvalues: Vec::new(),
            parent_functions: Vec::new(),
            blocks: Vec::new(),
            allocator,
            string_pool,
        }
    }

    /// 从宿主 UTF-8 文本源名生成字节码。
    ///
    /// Lua 提供的 chunk name 应使用 [`Self::generate_with_source_bytes`]，
    /// 以免任意字节在文本边界被重编码。
    pub fn generate(self, chunk: &Chunk, source_name: &str) -> Result<Proto, CodegenError> {
        self.generate_with_source_bytes(chunk, source_name.as_bytes())
    }

    /// 从任意 Lua 字节序列源名生成字节码。
    pub fn generate_with_source_bytes(
        mut self,
        chunk: &Chunk,
        source_name: &[u8],
    ) -> Result<Proto, CodegenError> {
        let source = self
            .allocator
            .allocate_string(self.string_pool, source_name);
        self.builder.set_source(Some(source));
        self.builder.set_vararg(true);

        self.emit_block(&chunk.statements)
            .map_err(|msg| CodegenError::new(msg, 0, 0))?;

        let final_line = chunk.statements.last().map(|s| s.end_line()).unwrap_or(1);
        self.code_abc(OpCode::RETURN, 0, 1, 0, final_line);

        // Lua 5.1 keeps a two-slot minimum, then records the exact highest
        // register boundary reached during lowering.
        let max_stack = self
            .reg_alloc
            .max_used()
            .max(self.active_var_count)
            .max(self.builder.max_stack_size() as i32)
            .max(2);
        self.builder.set_max_stack_size(max_stack as u8);
        self.attach_local_debug();

        Ok(self.builder.into_proto())
    }

    pub(crate) fn add_string_constant(&mut self, value: &str) -> Option<i32> {
        self.add_byte_string_constant(value.as_bytes())
    }

    pub(crate) fn add_byte_string_constant(&mut self, value: &[u8]) -> Option<i32> {
        let string = self.allocator.allocate_string(self.string_pool, value);
        Some(self.builder.add_gc_string_constant(string))
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

// Default is intentionally unavailable: every compiler is tied to explicit
// allocation services for the complete code-generation pass.
