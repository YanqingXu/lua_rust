//! 函数编译器 (Function Compiler)
//!
//! 管理子函数原型的编译生命周期：参数绑定、函数体编译、
//! upvalue 闭包元数据和调试信息附加。
//!
//! C++ 参考: `lua_cpp/src/compiler/codegen/function_compiler.hpp/.cpp`

use lua_core::proto::Proto;

use crate::codegen::CodeGenerator;
use crate::codegen::types::{CompiledFunction, UpvalueCapture};
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
        // 创建子 Proto
        let mut sub_proto = Proto::new();
        sub_proto.set_vararg(is_vararg);
        sub_proto.set_max_stack_size(2);
        sub_proto.set_line_defined(linedefined);

        // 保存当前状态
        let saved_builder = std::mem::replace(
            &mut self.builder,
            crate::codegen::builder::BytecodeBuilder::new(sub_proto),
        );
        let saved_reg_alloc = std::mem::replace(
            &mut self.reg_alloc,
            crate::codegen::RegisterAllocator::new(0),
        );
        let saved_local_vars = std::mem::take(&mut self.local_vars);
        let saved_active_vars = self.active_var_count;
        self.active_var_count = 0;
        let saved_upvalues = std::mem::take(&mut self.upvalues);
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

        // 编译函数体
        self.emit_block(body)?;

        // 末尾兜底 RETURN
        self.code_abc(OpCode::RETURN, 0, 1, 0, self.current_line);

        // 附加调试信息
        self.attach_local_debug();

        // 保存编译结果
        let upvalues = self.upvalues.clone();
        let proto = std::mem::replace(&mut self.builder, saved_builder);
        let _proto = proto.into_proto();

        // 添加子 Proto 到父 Proto
        // TODO: Create GcRef properly through GC
        // For now use a placeholder index
        let proto_index = 0i32; // placeholder

        // 恢复状态
        self.reg_alloc = saved_reg_alloc;
        self.local_vars = saved_local_vars;
        self.active_var_count = saved_active_vars;
        self.upvalues = saved_upvalues;
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
