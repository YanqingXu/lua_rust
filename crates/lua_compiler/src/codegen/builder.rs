//! 字节码构建器
//!
//! 封装对当前 Proto 的写操作：指令发射、行号信息、调试信息。
//! 常量管理直接调用 Proto::add_constant()。
//!

use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::{Instruction, Proto};
use lua_core::string_pool::StringPool;
use lua_core::value::Value;

use crate::opcode::{self, OpCode};

/// 字节码构建器
///
/// 封装对当前 Proto 的写操作：字节码指令、行号信息、子原型和调试名称。
///
/// CodeGen 降低层的代码决定发射什么指令；
/// Builder 负责 Proto 的底层数据操作。
///
pub struct BytecodeBuilder {
    /// 当前绑定的 Proto
    proto: Proto,
    /// 字符串驻留池（用于字符串驻留；mut 因为 intern() 需要 &mut）
    string_pool: Option<*mut StringPool>,
}

// SAFETY: string_pool raw pointer is only accessed during compilation,
// which is single-threaded. The original StringPool outlives the builder.
unsafe impl Send for BytecodeBuilder {}
// SAFETY: See Send impl.
unsafe impl Sync for BytecodeBuilder {}

impl BytecodeBuilder {
    /// 创建新的 BytecodeBuilder
    pub fn new(proto: Proto) -> Self {
        Self {
            proto,
            string_pool: None,
        }
    }

    /// 绑定 StringPool（用于字符串驻留）
    pub fn bind_pool(&mut self, pool: &mut StringPool) {
        self.string_pool = Some(pool as *mut StringPool);
    }

    /// 获取 Proto 的可变引用
    pub fn proto_mut(&mut self) -> &mut Proto {
        &mut self.proto
    }

    /// 获取 Proto 的不可变引用
    pub fn proto(&self) -> &Proto {
        &self.proto
    }

    /// 消耗 Builder，返回 Proto
    pub fn into_proto(self) -> Proto {
        self.proto
    }

    /// 获取当前指令数量
    pub fn instruction_count(&self) -> usize {
        self.proto.instruction_count()
    }

    /// 检查是否有指令
    pub fn has_instructions(&self) -> bool {
        self.instruction_count() > 0
    }

    /// 获取最后一条指令的操作码
    pub fn last_opcode(&self) -> Option<OpCode> {
        let count = self.instruction_count();
        if count == 0 {
            return None;
        }
        let inst = self.proto.instruction(count - 1);
        Some(opcode::get_opcode(inst))
    }

    // ── 指令发射 ──────────────────────────────────────────────────

    /// 发射 iABC 格式指令，返回指令 PC
    pub fn emit_abc(&mut self, line: i32, op: OpCode, a: i32, b: i32, c: i32) -> i32 {
        let inst = opcode::create_abc(op, a, b, c);
        self.emit(line, inst)
    }

    /// 发射 iABx 格式指令，返回指令 PC
    pub fn emit_abx(&mut self, line: i32, op: OpCode, a: i32, bx: i32) -> i32 {
        let inst = opcode::create_abx(op, a, bx);
        self.emit(line, inst)
    }

    /// 发射 iAsBx 格式指令，返回指令 PC
    pub fn emit_as_bx(&mut self, line: i32, op: OpCode, a: i32, sbx: i32) -> i32 {
        let inst = opcode::create_as_bx(op, a, sbx);
        self.emit(line, inst)
    }

    /// 发射原始指令，返回指令 PC
    pub fn emit_raw(&mut self, line: i32, inst: Instruction) -> i32 {
        self.emit(line, inst)
    }

    // ── 指令访问 ──────────────────────────────────────────────────

    /// 获取指定 PC 位置的指令
    pub fn instruction(&self, pc: i32) -> Option<Instruction> {
        if pc < 0 {
            return None;
        }
        let idx = pc as usize;
        if idx < self.proto.instruction_count() {
            Some(self.proto.instruction(idx))
        } else {
            None
        }
    }

    /// 替换指定 PC 位置的指令
    pub fn replace_instruction(&mut self, pc: i32, inst: Instruction) -> bool {
        if pc < 0 {
            return false;
        }
        let idx = pc as usize;
        if idx < self.proto.instruction_count() {
            self.proto.set_instruction(idx, inst);
            true
        } else {
            false
        }
    }

    // ── 常量管理（委托给 Proto）──────────────────────────────────

    /// 添加数字常量，返回常量索引
    pub fn add_number_constant(&mut self, value: f64) -> i32 {
        self.proto.add_constant(Value::Number(value)) as i32
    }

    /// 添加布尔常量，返回常量索引
    pub fn add_bool_constant(&mut self, value: bool) -> i32 {
        self.proto.add_constant(Value::Boolean(value)) as i32
    }

    /// 添加 nil 常量，返回常量索引
    pub fn add_nil_constant(&mut self) -> i32 {
        self.proto.add_constant(Value::Nil) as i32
    }

    /// 添加字符串常量，返回常量索引
    ///
    /// 如果 StringPool 已绑定，则优先从驻留池获取已有字符串（保证相同内容的
    /// 字符串获得相同的 GcRef，从而使指针比较在表查找中直接生效）。
    /// 如果池中不存在，则创建新 GcString 并注册到驻留池。
    pub fn add_string_constant(
        &mut self,
        gc: &mut lua_core::gc::collector::GarbageCollector,
        value: &str,
    ) -> Option<i32> {
        // Try StringPool interning first (for pointer-identical GcRef across
        // compiler and stdlib).
        let gc_str = if let Some(pool_ptr) = self.string_pool {
            // SAFETY: pool_ptr was set from a valid &mut StringPool that outlives
            // the builder. Compilation is single-threaded.
            let pool: &mut StringPool = unsafe { &mut *pool_ptr };
            pool.intern(gc, value)
        } else {
            // No pool available — fall back to direct GC allocation
            gc.create(GcString::new(value))
        };
        let idx = self.proto.add_constant(Value::String(gc_str)) as i32;
        Some(idx)
    }

    /// 添加子原型，返回原型索引
    pub fn add_sub_proto(&mut self, proto_ref: GcRef<Proto>) -> i32 {
        self.proto.add_proto(proto_ref) as i32
    }

    // ── 调试信息 ──────────────────────────────────────────────────

    /// 设置源文件名
    pub fn set_source(&mut self, source: Option<GcRef<GcString>>) {
        self.proto.set_source(source);
    }

    /// 添加局部变量调试信息
    pub fn add_local_debug(
        &mut self,
        varname: Option<GcRef<GcString>>,
        startpc: i32,
        endpc: i32,
        reg: i32,
    ) {
        self.proto.add_loc_var(varname, startpc, endpc, reg);
    }

    /// 添加上值名称
    pub fn add_upvalue_name(&mut self, name: GcRef<GcString>) -> i32 {
        self.proto.add_upvalue_name(name) as i32
    }

    /// 设置上值数量
    pub fn set_num_upvalues(&mut self, n: u8) {
        self.proto.set_num_upvalues(n);
    }

    /// 设置参数数量
    pub fn set_num_params(&mut self, n: u8) {
        self.proto.set_num_params(n);
    }

    /// 设置可变参数标志
    pub fn set_vararg(&mut self, flag: bool) {
        self.proto.set_vararg(flag);
    }

    /// 设置 Lua 5.1 可变参数标志位
    pub fn set_vararg_flags(&mut self, flags: u8) {
        self.proto.set_vararg_flags(flags);
    }

    /// 获取最大栈大小
    pub fn max_stack_size(&self) -> u8 {
        self.proto.max_stack_size()
    }

    /// 设置最大栈大小
    pub fn set_max_stack_size(&mut self, size: u8) {
        self.proto.set_max_stack_size(size);
    }

    // ── 内部方法 ──────────────────────────────────────────────────

    fn emit(&mut self, line: i32, inst: Instruction) -> i32 {
        let pc = self.proto.instruction_count() as i32;
        self.proto.add_instruction(inst);
        self.proto.add_line_info(line);
        pc
    }
}
