//! CodeGen 核心类型定义
//!
//! 定义字节码生成过程中使用的数据流类型：
//! PatchList, ValueResult, CondResult, LValueRef, SymbolRef, CallResultInfo
//!

// =====================================================================
// 常量
// =====================================================================

/// 无效跳转标记
pub const NO_JUMP: i32 = -1;

// =====================================================================
// PatchList — 跳转修补链表
// =====================================================================

/// 跳转修补链表
///
/// 记录待回填的跳转指令位置列表。
///
#[derive(Debug, Clone)]
pub struct PatchList {
    pub pcs: Vec<i32>,
}

impl PatchList {
    pub fn new() -> Self {
        Self { pcs: Vec::new() }
    }

    pub fn empty(&self) -> bool {
        self.pcs.is_empty()
    }

    pub fn size(&self) -> usize {
        self.pcs.len()
    }

    pub fn front(&self) -> i32 {
        self.pcs.first().copied().unwrap_or(NO_JUMP)
    }

    pub fn clear(&mut self) {
        self.pcs.clear();
    }

    pub fn append(&mut self, pc: i32) {
        if pc != NO_JUMP {
            self.pcs.push(pc);
        }
    }

    pub fn append_list(&mut self, other: &PatchList) {
        self.pcs.extend_from_slice(&other.pcs);
    }

    pub fn merge(mut lhs: PatchList, rhs: &PatchList) -> PatchList {
        lhs.append_list(rhs);
        lhs
    }
}

impl Default for PatchList {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// ValueResult — 右值结果
// =====================================================================

/// 右值结果的访问类型
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    None,
    Local,
    Upvalue,
    Global,
    Indexed,
    Call,
    Vararg,
}

/// 即时值的类型
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImmediateKind {
    None,
    Nil,
    Boolean,
    Number,
}

/// 右值结果
///
/// 表示表达式的求值结果，支持多种形式：即时值、常量引用、寄存器引用、
/// 待加载、可重定位、多返回值和待跳转。
///
#[derive(Debug, Clone, Default)]
pub enum ValueResult {
    /// 无结果
    #[default]
    None,
    /// 即时值（nil / bool / number）
    Immediate {
        kind: ImmediateKind,
        boolean_value: bool,
        number_value: f64,
    },
    /// 常量表引用
    ConstantRef { const_index: i32 },
    /// 寄存器引用
    RegisterRef {
        reg: i32,
        owns_register: bool,
        access: AccessKind,
    },
    /// 待加载（需后续指令加载）
    PendingLoad {
        access: AccessKind,
        reg: i32,
        const_index: i32,
        aux: i32,
    },
    /// 可重定位（指令 PC 待回填）
    Relocatable { instruction_pc: i32 },
    /// 多返回值
    MultiRet {
        access: AccessKind,
        reg: i32,
        instruction_pc: i32,
    },
    /// 待跳转
    PendingJump { instruction_pc: i32 },
}

impl ValueResult {
    // ── 构造方法 ──────────────────────────────────────────────────

    pub fn make_nil() -> Self {
        ValueResult::Immediate {
            kind: ImmediateKind::Nil,
            boolean_value: false,
            number_value: 0.0,
        }
    }

    pub fn make_boolean(value: bool) -> Self {
        ValueResult::Immediate {
            kind: ImmediateKind::Boolean,
            boolean_value: value,
            number_value: 0.0,
        }
    }

    pub fn make_number(value: f64) -> Self {
        ValueResult::Immediate {
            kind: ImmediateKind::Number,
            boolean_value: false,
            number_value: value,
        }
    }

    pub fn make_constant(index: i32) -> Self {
        ValueResult::ConstantRef { const_index: index }
    }

    pub fn make_register(index: i32, owns: bool, access: AccessKind) -> Self {
        ValueResult::RegisterRef {
            reg: index,
            owns_register: owns,
            access,
        }
    }

    pub fn make_pending_load(
        access: AccessKind,
        source_reg: i32,
        constant_index: i32,
        aux_index: i32,
    ) -> Self {
        ValueResult::PendingLoad {
            access,
            reg: source_reg,
            const_index: constant_index,
            aux: aux_index,
        }
    }

    pub fn make_relocatable(pc: i32) -> Self {
        ValueResult::Relocatable { instruction_pc: pc }
    }

    pub fn make_multi_ret(access: AccessKind, base_reg: i32, pc: i32) -> Self {
        ValueResult::MultiRet {
            access,
            reg: base_reg,
            instruction_pc: pc,
        }
    }

    pub fn make_pending_jump(pc: i32) -> Self {
        ValueResult::PendingJump { instruction_pc: pc }
    }
}

// =====================================================================
// CondResult — 条件求值结果
// =====================================================================

/// 条件求值结果
///
/// 记录条件表达式的真/假跳转链表。
///
#[derive(Debug, Clone)]
pub struct CondResult {
    pub true_list: PatchList,
    pub false_list: PatchList,
    pub known_constant: bool,
    pub constant_value: bool,
}

impl CondResult {
    pub fn new() -> Self {
        Self {
            true_list: PatchList::new(),
            false_list: PatchList::new(),
            known_constant: false,
            constant_value: false,
        }
    }

    pub fn make_constant(value: bool) -> Self {
        Self {
            true_list: PatchList::new(),
            false_list: PatchList::new(),
            known_constant: true,
            constant_value: value,
        }
    }
}

impl Default for CondResult {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// LValueRef — 左值引用
// =====================================================================

/// 左值引用的类型
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LValueKind {
    None,
    Local,
    Upvalue,
    Global,
    Indexed,
}

/// 左值引用
///
/// 表示可被赋值的存储位置（变量、上值、全局或索引表达式）。
///
#[derive(Debug, Clone)]
pub struct LValueRef {
    pub kind: LValueKind,
    /// Local: 寄存器槽位; Upvalue: upvalue 索引; Global: 常量索引
    pub slot: i32,
    pub table_reg: i32,
    pub key: i32,
    pub aux: i32,
}

impl LValueRef {
    pub fn new() -> Self {
        Self {
            kind: LValueKind::None,
            slot: -1,
            table_reg: -1,
            key: -1,
            aux: -1,
        }
    }

    pub fn valid(&self) -> bool {
        self.kind != LValueKind::None
    }
}

impl Default for LValueRef {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// SymbolRef — 名字绑定结果
// =====================================================================

/// 符号类型
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    None,
    Local,
    Upvalue,
    Global,
}

/// 名字解析结果
///
/// 将 NameExpr 中的名字解析为 Local / Upvalue / Global 绑定。
///
#[derive(Debug, Clone)]
pub struct SymbolRef {
    pub kind: SymbolKind,
    /// Local: 寄存器槽位; Upvalue: upvalue 索引; Global: 字符串常量索引
    pub index: i32,
    /// 原始名字
    pub name: String,
}

impl SymbolRef {
    pub fn new(kind: SymbolKind, index: i32, name: impl Into<String>) -> Self {
        Self {
            kind,
            index,
            name: name.into(),
        }
    }

    pub fn valid(&self) -> bool {
        self.kind != SymbolKind::None
    }
}

impl Default for SymbolRef {
    fn default() -> Self {
        Self {
            kind: SymbolKind::None,
            index: -1,
            name: String::new(),
        }
    }
}

// =====================================================================
// CallResultInfo — 调用结果信息
// =====================================================================

/// 调用结果类型
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallResultKind {
    None,
    Call,
    Vararg,
}

/// 调用结果信息
///
#[derive(Debug, Clone)]
pub struct CallResultInfo {
    pub kind: CallResultKind,
    pub base_reg: i32,
    pub instruction_pc: i32,
    pub open_multi_ret: bool,
}

impl CallResultInfo {
    pub fn new() -> Self {
        Self {
            kind: CallResultKind::None,
            base_reg: -1,
            instruction_pc: NO_JUMP,
            open_multi_ret: false,
        }
    }

    pub fn valid(&self) -> bool {
        self.kind != CallResultKind::None
    }
}

impl Default for CallResultInfo {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// CompiledFunction — 编译完成的函数
// =====================================================================

/// 编译完成的函数信息
///
#[derive(Debug, Clone)]
pub struct CompiledFunction {
    pub proto_index: i32,
    pub upvalues: Vec<UpvalueCapture>,
}

/// Upvalue 捕获信息
///
#[derive(Debug, Clone)]
pub struct UpvalueCapture {
    pub name: String,
    pub in_stack: bool,
    pub index: i32,
}

impl UpvalueCapture {
    pub fn new(name: impl Into<String>, in_stack: bool, index: i32) -> Self {
        Self {
            name: name.into(),
            in_stack,
            index,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParentFunctionContext {
    pub local_vars: Vec<LocalVar>,
    pub upvalues: Vec<UpvalueCapture>,
}

impl ParentFunctionContext {
    pub fn new(local_vars: Vec<LocalVar>, upvalues: Vec<UpvalueCapture>) -> Self {
        Self {
            local_vars,
            upvalues,
        }
    }
}

// =====================================================================
// LocalVar — 局部变量信息
// =====================================================================

/// 局部变量调试信息
///
#[derive(Debug, Clone)]
pub struct LocalVar {
    pub name: String,
    pub reg: i32,
    pub startpc: i32,
    pub endpc: i32,
    pub captured: bool,
}

impl LocalVar {
    pub fn new(name: impl Into<String>, reg: i32, startpc: i32) -> Self {
        Self {
            name: name.into(),
            reg,
            startpc,
            endpc: -1,
            captured: false,
        }
    }
}

// =====================================================================
// BlockInfo — 代码块信息
// =====================================================================

/// 代码块信息（用于 break 跳转管理）
///
#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub active_var_count: i32,
    pub breaklist: PatchList,
    pub is_breakable: bool,
}
