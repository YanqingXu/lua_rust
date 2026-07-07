//! 跳转修补器 (Jump Patcher)
//!
//! 管理跳转指令的创建、链表编码和回填（backpatching）。
//!
//! 未解析的 JMP 指令编码为单向链表：sBx 字段指向下一条未解析的
//! 跳转 PC，最终 patching 时统一回填真实目标地址。
//!

use crate::codegen::CodeGenerator;
use crate::codegen::types::{NO_JUMP, PatchList};
use crate::opcode::{self, MAXARG_SBX, NO_REG, OpCode};

impl CodeGenerator {
    // ── 跳转发射 ──────────────────────────────────────────────────

    /// 发射无条件跳转，返回跳转 PC
    ///
    pub fn emit_jump(&mut self) -> i32 {
        let pending = self.jpc;
        self.jpc = NO_JUMP;
        let jump_pc = self
            .builder
            .emit_as_bx(self.current_line, OpCode::JMP, 0, NO_JUMP);
        self.concat_jump_list(jump_pc, pending);
        jump_pc
    }

    /// 发射条件跳转（指令 + 后续 JMP），返回跳转 PC
    ///
    pub fn emit_conditional_jump(&mut self, mut op: OpCode, mut a: i32, b: i32, c: i32) -> i32 {
        if op == OpCode::TESTSET && a == NO_REG {
            op = OpCode::TEST;
            a = b;
            // b = 0 (unused in TEST with c mode)
        }

        self.flush_pending_jumps();
        self.builder.emit_abc(self.current_line, op, a, b, c);
        self.emit_jump()
    }

    // ── 跳转链表回填 ─────────────────────────────────────────────

    /// 将整条跳转链表回填到目标 PC
    ///
    pub fn patch_list(&mut self, mut list: i32, target: i32) {
        while list != NO_JUMP {
            let next = self.get_jump(list);
            self.fix_jump(list, target);
            list = next;
        }
    }

    /// 将 PatchList 回填到目标 PC
    ///
    pub fn patch_list_vec(&mut self, list: &PatchList, target: i32) {
        for &pc in &list.pcs {
            self.fix_jump(pc, target);
        }
    }

    /// 将跳转链表回填到当前位置
    ///
    pub fn patch_to_here(&mut self, list: i32) {
        let target = self.builder.instruction_count() as i32;
        self.concat_jump_list_into_jpc(list);
        self.patch_list(self.jpc, target);
        self.jpc = NO_JUMP;
    }

    /// 将 PatchList 回填到当前位置
    ///
    pub fn patch_list_to_here(&mut self, list: &PatchList) {
        let target = self.builder.instruction_count() as i32;
        self.patch_list_vec(list, target);
    }

    /// 刷新待处理的跳转到当前指令位置
    ///
    pub fn flush_pending_jumps(&mut self) {
        let target = self.builder.instruction_count() as i32;
        self.patch_list(self.jpc, target);
        self.jpc = NO_JUMP;
    }

    /// 将右侧跳转链表连接到左侧链表的末尾
    ///
    pub fn concat_jump_list(&mut self, left: i32, right: i32) -> i32 {
        if right == NO_JUMP {
            return left;
        }
        if left == NO_JUMP {
            return right;
        }

        let mut list = left;
        loop {
            let next = self.get_jump(list);
            if next == NO_JUMP {
                break;
            }
            list = next;
        }
        self.fix_jump(list, right);
        left
    }

    fn concat_jump_list_into_jpc(&mut self, list: i32) {
        self.jpc = self.concat_jump_list(self.jpc, list);
    }

    /// 获取当前标签（下一条指令的 PC）
    ///
    pub fn get_label(&self) -> i32 {
        self.builder.instruction_count() as i32
    }

    // ── 跳转链表编码 ─────────────────────────────────────────────

    /// 从跳转指令中读取链表下一个节点
    ///
    /// 跳转的 sBx 编码的是到下一个跳转 PC 的偏移（而非最终目标）。
    ///
    fn get_jump(&self, pc: i32) -> i32 {
        let inst = match self.builder.instruction(pc) {
            Some(i) => i,
            None => return NO_JUMP,
        };
        let offset = opcode::get_arg_sbx(inst);
        if offset == NO_JUMP {
            return NO_JUMP;
        }
        (pc + 1) + offset
    }

    /// 修正跳转指令的目标地址
    ///
    pub fn fix_jump(&mut self, pc: i32, dest: i32) {
        let mut jump = match self.builder.instruction(pc) {
            Some(i) => i,
            None => return,
        };
        let offset = dest - (pc + 1);
        // 跳转偏移范围检查
        if !(-MAXARG_SBX..=MAXARG_SBX).contains(&offset) {
            // TODO: emit proper CodegenError instead of panic
            panic!("control structure too long");
        }
        opcode::set_arg_sbx(&mut jump, offset);
        self.builder.replace_instruction(pc, jump);
    }

    /// 收集链表为 PatchList
    ///
    pub fn collect_patch_list(&self, mut list: i32) -> PatchList {
        let mut result = PatchList::new();
        while list != NO_JUMP {
            result.append(list);
            list = self.get_jump(list);
        }
        result
    }
}
