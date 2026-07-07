//! 名字绑定器 (Name Binder)
//!
//! 将 AST 中的 NameExpr 解析为 SymbolRef（Local → Upvalue → Global 三阶段查找），
//! 并提供 SymbolRef → ValueResult / LValueRef 的转换。
//!

use crate::codegen::CodeGenerator;
use crate::codegen::types::{
    AccessKind, LValueKind, LValueRef, SymbolKind, SymbolRef, ValueResult,
};

impl CodeGenerator {
    // ── 名字解析 ──────────────────────────────────────────────────

    /// 解析名字到 SymbolRef（Local → Upvalue → Global 三阶段查找）
    ///
    pub fn resolve_name(&mut self, name: &str) -> SymbolRef {
        // 1. 查找局部变量
        let reg = self.find_local_var(name);
        if reg >= 0 {
            return SymbolRef::new(SymbolKind::Local, reg, name);
        }

        // 2. 解析 upvalue
        let up = self.resolve_upvalue(name);
        if up >= 0 {
            return SymbolRef::new(SymbolKind::Upvalue, up, name);
        }

        // 3. Fallback: 全局变量
        // SAFETY: self.gc is set during CodeGenerator::new() from a valid &mut
        // GarbageCollector reference that outlives the compilation process.
        let gc: &mut lua_core::gc::collector::GarbageCollector = unsafe { &mut *self.gc };
        let const_idx = self.builder.add_string_constant(gc, name).unwrap_or(-1);
        SymbolRef::new(SymbolKind::Global, const_idx, name)
    }

    /// 将 SymbolRef 转为 ValueResult（读路径）
    ///
    pub fn symbol_to_value(&self, sym: &SymbolRef) -> ValueResult {
        match sym.kind {
            SymbolKind::Local => ValueResult::make_register(sym.index, false, AccessKind::Local),
            SymbolKind::Upvalue => {
                ValueResult::make_pending_load(AccessKind::Upvalue, -1, -1, sym.index)
            }
            SymbolKind::Global => {
                ValueResult::make_pending_load(AccessKind::Global, -1, sym.index, -1)
            }
            SymbolKind::None => ValueResult::None,
        }
    }

    /// 将 SymbolRef 转为 LValueRef（写路径）
    ///
    pub fn symbol_to_lvalue(&self, sym: &SymbolRef) -> LValueRef {
        let mut result = LValueRef::new();
        match sym.kind {
            SymbolKind::Local => {
                result.kind = LValueKind::Local;
                result.slot = sym.index;
            }
            SymbolKind::Upvalue => {
                result.kind = LValueKind::Upvalue;
                result.slot = sym.index;
            }
            SymbolKind::Global => {
                result.kind = LValueKind::Global;
                result.slot = sym.index;
            }
            SymbolKind::None => {}
        }
        result
    }
}
