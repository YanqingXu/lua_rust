//! 作用域管理器 (Scope Manager)
//!
//! 管理局部变量激活/移除、代码块栈、break 跳转列表、
//! upvalue 注册和 CLOSE 指令发射。
//!
//! C++ 参考: `lua_cpp/src/compiler/codegen/scope_manager.hpp/.cpp`

use crate::codegen::CodeGenerator;
use crate::codegen::types::{BlockInfo, LocalVar, PatchList, UpvalueCapture};
use crate::opcode::OpCode;
use lua_core::gc_string::GcString;

impl CodeGenerator {
    // ── 局部变量 ──────────────────────────────────────────────────

    /// 添加局部变量，返回分配的寄存器槽位
    ///
    /// C++ 对应: `ScopeManager::addLocalVar()`
    pub fn add_local_var(&mut self, name: impl Into<String>) -> i32 {
        let reg = self.reg_alloc.current();
        let startpc = self.builder.instruction_count() as i32;
        self.local_vars.push(LocalVar::new(name, reg, startpc));
        self.reg_alloc.reserve(1);
        let current_max = self.builder.max_stack_size() as i32;
        let new_max = self.reg_alloc.check_stack(0, current_max);
        self.builder.set_max_stack_size(new_max as u8);
        reg
    }

    /// 查找局部变量，返回寄存器槽位（-1 表示未找到）
    ///
    /// C++ 对应: `ScopeManager::findLocalVar()`
    pub fn find_local_var(&self, name: &str) -> i32 {
        for var in self.local_vars.iter().rev() {
            if var.name == name && var.endpc == -1 {
                return var.reg;
            }
        }
        -1
    }

    /// 标记局部变量被捕获（用于 upvalue）
    ///
    /// C++ 对应: `ScopeManager::markLocalCaptured()`
    pub fn mark_local_captured(&mut self, reg: i32) {
        for var in self.local_vars.iter_mut().rev() {
            if var.reg == reg && var.endpc == -1 {
                var.captured = true;
                return;
            }
        }
    }

    /// 调整活动局部变量计数
    ///
    /// C++ 对应: `ScopeManager::adjustLocalVars()`
    pub fn adjust_local_vars(&mut self, count: i32) {
        self.active_var_count += count;
        self.reg_alloc.reset_to_locals(self.active_var_count);
        let current_max = self.builder.max_stack_size() as i32;
        let new_max = self.reg_alloc.check_stack(0, current_max);
        self.builder.set_max_stack_size(new_max as u8);
    }

    /// 移除局部变量到指定层级
    ///
    /// C++ 对应: `ScopeManager::removeLocalVars()`
    pub fn remove_local_vars(&mut self, to_level: i32) -> Option<i32> {
        let close_pc = self.close_scope_upvalues(to_level);
        let pc = self.builder.instruction_count() as i32;

        // 关闭离开作用域的局部变量
        while self.active_var_count > to_level {
            self.active_var_count -= 1;
            for var in self.local_vars.iter_mut().rev() {
                if var.endpc == -1 {
                    var.endpc = pc;
                    break;
                }
            }
        }

        self.reg_alloc.reset_to_locals(self.active_var_count);
        let current_max = self.builder.max_stack_size() as i32;
        let new_max = self.reg_alloc.check_stack(0, current_max);
        self.builder.set_max_stack_size(new_max as u8);
        close_pc
    }

    /// 关闭被捕获的局部变量（发射 CLOSE 指令）
    ///
    /// C++ 对应: `ScopeManager::closeScopeUpvalues()`
    pub(crate) fn close_scope_upvalues(&mut self, level: i32) -> Option<i32> {
        if self.active_var_count <= level {
            return None;
        }

        // 检查是否有被捕获的局部变量
        let has_captured = self
            .local_vars
            .iter()
            .any(|v| v.endpc == -1 && v.reg >= level && v.captured);
        if !has_captured {
            return None;
        }

        // 如果最后一条指令是 RETURN，跳过（返回时会自动关闭）
        if self.builder.has_instructions() && self.builder.last_opcode() == Some(OpCode::RETURN) {
            return None;
        }

        Some(self.emit_close(level))
    }

    /// 发射 CLOSE 指令
    fn emit_close(&mut self, level: i32) -> i32 {
        self.flush_pending_jumps();
        self.builder
            .emit_abc(self.current_line, OpCode::CLOSE, level, 0, 0)
    }

    /// 活动局部变量数量
    pub fn active_local_count(&self) -> i32 {
        self.active_var_count
    }

    /// 局部变量列表（只读）
    pub fn local_vars_slice(&self) -> &[LocalVar] {
        &self.local_vars
    }

    // ── Upvalue 管理 ──────────────────────────────────────────────

    /// 在当前函数中查找 upvalue
    pub fn find_upvalue(&self, name: &str) -> i32 {
        for (i, uv) in self.upvalues.iter().enumerate() {
            if uv.name == name {
                return i as i32;
            }
        }
        -1
    }

    /// 添加 upvalue
    pub fn add_upvalue(&mut self, name: impl Into<String>, in_stack: bool, index: i32) -> i32 {
        let name = name.into();
        if let Some(pos) = self.upvalues.iter().position(|uv| uv.name == name) {
            return pos as i32;
        }
        self.upvalues
            .push(UpvalueCapture::new(name, in_stack, index));
        (self.upvalues.len() - 1) as i32
    }

    /// 解析 upvalue（跨函数查找）
    ///
    /// C++ 对应: `ScopeManager::resolveUpvalue()`
    pub fn resolve_upvalue(&mut self, name: &str) -> i32 {
        if self.parent_functions.is_empty() {
            return -1;
        }

        let parent_idx = self.parent_functions.len() - 1;
        if let Some(local) = find_context_local(&self.parent_functions[parent_idx], name) {
            mark_context_local_captured(&mut self.parent_functions[parent_idx], local);
            return self.add_upvalue(name, true, local);
        }

        if let Some(parent_upvalue) =
            resolve_context_upvalue(&mut self.parent_functions, parent_idx, name)
        {
            return self.add_upvalue(name, false, parent_upvalue);
        }

        -1
    }

    /// Upvalue 列表
    pub fn upvalues_slice(&self) -> &[UpvalueCapture] {
        &self.upvalues
    }

    // ── 代码块管理 ────────────────────────────────────────────────

    /// 进入代码块
    ///
    /// C++ 对应: `ScopeManager::enterBlock()`
    pub fn enter_block(&mut self, is_breakable: bool) {
        let block = BlockInfo {
            active_var_count: self.active_var_count,
            breaklist: PatchList::new(),
            is_breakable,
        };
        self.blocks.push(block);
    }

    /// 离开代码块
    ///
    /// C++ 对应: `ScopeManager::leaveBlock()`
    pub fn leave_block(&mut self) {
        let block = self.blocks.pop().expect("No block to leave");
        let close_pc = self.remove_local_vars(block.active_var_count);
        let label = self.get_label();
        self.patch_list_vec(&block.breaklist, close_pc.unwrap_or(label));
    }

    /// 当前代码块
    pub fn current_block(&self) -> Option<&BlockInfo> {
        self.blocks.last()
    }

    /// 查找最近的 breakable 块
    pub fn find_breakable_block(&self) -> Option<&BlockInfo> {
        self.blocks.iter().rev().find(|b| b.is_breakable)
    }

    /// 向代码块的 breaklist 追加跳转
    pub fn append_break_jump(&mut self, jump_pc: i32) {
        if let Some(block) = self.blocks.iter_mut().rev().find(|b| b.is_breakable) {
            let mut combined = PatchList::new();
            combined.append_list(&block.breaklist);
            combined.append(jump_pc);
            block.breaklist = combined;
        }
    }

    // ── 调试信息 ──────────────────────────────────────────────────

    /// 附加局部变量调试信息到 Proto
    pub fn attach_local_debug(&mut self) {
        for var in &self.local_vars {
            let endpc = if var.endpc >= 0 {
                var.endpc
            } else {
                self.builder.instruction_count() as i32
            };
            // SAFETY: self.gc is set by CodeGenerator::new and lives for this codegen pass.
            let gc = unsafe { &mut *self.gc };
            let name = gc.create(GcString::new(&var.name));
            self.builder
                .add_local_debug(Some(name), var.startpc, endpc, var.reg);
        }
    }
}

fn find_context_local(
    ctx: &crate::codegen::types::ParentFunctionContext,
    name: &str,
) -> Option<i32> {
    ctx.local_vars
        .iter()
        .rev()
        .find(|var| var.name == name && var.endpc == -1)
        .map(|var| var.reg)
}

fn mark_context_local_captured(ctx: &mut crate::codegen::types::ParentFunctionContext, reg: i32) {
    if let Some(var) = ctx
        .local_vars
        .iter_mut()
        .rev()
        .find(|var| var.reg == reg && var.endpc == -1)
    {
        var.captured = true;
    }
}

fn context_add_upvalue(
    ctx: &mut crate::codegen::types::ParentFunctionContext,
    name: &str,
    in_stack: bool,
    index: i32,
) -> i32 {
    if let Some(pos) = ctx.upvalues.iter().position(|uv| uv.name == name) {
        return pos as i32;
    }
    ctx.upvalues
        .push(UpvalueCapture::new(name, in_stack, index));
    (ctx.upvalues.len() - 1) as i32
}

fn resolve_context_upvalue(
    contexts: &mut [crate::codegen::types::ParentFunctionContext],
    current_idx: usize,
    name: &str,
) -> Option<i32> {
    if current_idx == 0 {
        return None;
    }

    let parent_idx = current_idx - 1;
    if let Some(local) = find_context_local(&contexts[parent_idx], name) {
        mark_context_local_captured(&mut contexts[parent_idx], local);
        return Some(context_add_upvalue(
            &mut contexts[current_idx],
            name,
            true,
            local,
        ));
    }

    let parent_upvalue = resolve_context_upvalue(contexts, parent_idx, name)?;
    Some(context_add_upvalue(
        &mut contexts[current_idx],
        name,
        false,
        parent_upvalue,
    ))
}
