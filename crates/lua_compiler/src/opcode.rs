//! Lua 5.1 虚拟机指令集定义
//!
//! 实现 Lua 5.1 虚拟机的完整指令集，包括操作码枚举、指令格式和编解码函数。
//!
//! ## 指令格式（32 位）
//! - **iABC**:  `[OP:6][A:8][C:9][B:9]` — 三操作数格式
//! - **iABx**:  `[OP:6][A:8][Bx:18]` — 两操作数格式（大索引）
//! - **iAsBx**: `[OP:6][A:8][sBx:18]` — 两操作数格式（有符号偏移）
//!

use lua_core::proto::Instruction;

// =====================================================================
// 指令格式枚举
// =====================================================================

/// 指令格式类型
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMode {
    /// 三操作数格式: OP(6) A(8) C(9) B(9)
    IABC,
    /// 两操作数格式（大索引）: OP(6) A(8) Bx(18)
    IABx,
    /// 两操作数格式（有符号偏移）: OP(6) A(8) sBx(18)
    IAsBx,
}

// =====================================================================
// 指令布局常量
// =====================================================================

/// C 字段位数
pub const SIZE_C: i32 = 9;
/// B 字段位数
pub const SIZE_B: i32 = 9;
/// Bx 字段位数 (= SIZE_C + SIZE_B)
pub const SIZE_BX: i32 = SIZE_C + SIZE_B; // 18
/// A 字段位数
pub const SIZE_A: i32 = 8;
/// OP 字段位数
pub const SIZE_OP: i32 = 6;

/// OP 字段起始位
pub const POS_OP: i32 = 0;
/// A 字段起始位 (= POS_OP + SIZE_OP)
pub const POS_A: i32 = POS_OP + SIZE_OP; // 6
/// C 字段起始位 (= POS_A + SIZE_A)
pub const POS_C: i32 = POS_A + SIZE_A; // 14
/// B 字段起始位 (= POS_C + SIZE_C)
pub const POS_B: i32 = POS_C + SIZE_C; // 23
/// Bx 字段起始位 (= POS_C)
pub const POS_BX: i32 = POS_C; // 14

/// Bx 字段最大值
pub const MAXARG_BX: i32 = (1 << SIZE_BX) - 1; // 262143
/// sBx 字段最大值（有符号）
pub const MAXARG_SBX: i32 = MAXARG_BX >> 1; // 131071
/// A 字段最大值
pub const MAXARG_A: i32 = (1 << SIZE_A) - 1; // 255
/// B 字段最大值
pub const MAXARG_B: i32 = (1 << SIZE_B) - 1; // 511
/// C 字段最大值
pub const MAXARG_C: i32 = (1 << SIZE_C) - 1; // 511

/// RK 寻址标志位（B 字段最高位）
pub const BITRK: i32 = 1 << (SIZE_B - 1); // 256
/// RK 常量索引最大值
pub const MAXINDEXRK: i32 = BITRK - 1; // 255

/// 特殊寄存器值（表示无寄存器 / 超出范围）
pub const NO_REG: i32 = MAXARG_A; // 255

/// 表构造器批处理大小
pub const LFIELDS_PER_FLUSH: i32 = 50;

// =====================================================================
// 操作码枚举（38 个指令）
// =====================================================================

/// Lua 5.1 虚拟机操作码（38 个指令）
///
/// discriminant 值固定为 Lua 5.1 的 38 条 opcode 编号（0..37）。
///
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum OpCode {
    // ── 数据移动指令 ──────────────────────────────────────────────
    /// R(A) := R(B)
    MOVE = 0,
    /// R(A) := K(Bx)
    LOADK = 1,
    /// R(A) := (Bool)B; if (C) pc++
    LOADBOOL = 2,
    /// R(A) := ... := R(B) := nil
    LOADNIL = 3,

    // ── 变量访问指令 ──────────────────────────────────────────────
    /// R(A) := UpValue[B]
    GETUPVAL = 4,
    /// R(A) := Gbl[K(Bx)]
    GETGLOBAL = 5,
    /// R(A) := R(B)[RK(C)]
    GETTABLE = 6,

    // ── 变量赋值指令 ──────────────────────────────────────────────
    /// Gbl[K(Bx)] := R(A)
    SETGLOBAL = 7,
    /// UpValue[B] := R(A)
    SETUPVAL = 8,
    /// R(A)[RK(B)] := RK(C)
    SETTABLE = 9,

    // ── 表操作指令 ────────────────────────────────────────────────
    /// R(A) := {} (size = B,C)
    NEWTABLE = 10,
    /// R(A+1) := R(B); R(A) := R(B)[RK(C)]
    SELF = 11,

    // ── 算术运算指令 ──────────────────────────────────────────────
    /// R(A) := RK(B) + RK(C)
    ADD = 12,
    /// R(A) := RK(B) - RK(C)
    SUB = 13,
    /// R(A) := RK(B) * RK(C)
    MUL = 14,
    /// R(A) := RK(B) / RK(C)
    DIV = 15,
    /// R(A) := RK(B) % RK(C)
    MOD = 16,
    /// R(A) := RK(B) ^ RK(C)
    POW = 17,
    /// R(A) := -R(B)
    UNM = 18,
    /// R(A) := not R(B)
    NOT = 19,
    /// R(A) := length of R(B)
    LEN = 20,

    // ── 字符串操作指令 ────────────────────────────────────────────
    /// R(A) := R(B).. ... ..R(C)
    CONCAT = 21,

    // ── 控制流指令 ────────────────────────────────────────────────
    /// pc += sBx
    JMP = 22,
    /// if ((RK(B) == RK(C)) ~= A) then pc++
    EQ = 23,
    /// if ((RK(B) <  RK(C)) ~= A) then pc++
    LT = 24,
    /// if ((RK(B) <= RK(C)) ~= A) then pc++
    LE = 25,
    /// if not (R(A) <=> C) then pc++
    TEST = 26,
    /// if (R(B) <=> C) then R(A) := R(B) else pc++
    TESTSET = 27,

    // ── 函数调用指令 ──────────────────────────────────────────────
    /// R(A), ... ,R(A+C-2) := R(A)(R(A+1), ... ,R(A+B-1))
    CALL = 28,
    /// return R(A)(R(A+1), ... ,R(A+B-1))
    TAILCALL = 29,
    /// return R(A), ... ,R(A+B-2)
    RETURN = 30,

    // ── 循环控制指令 ──────────────────────────────────────────────
    /// R(A)+=R(A+2); if R(A) <?= R(A+1) then { pc+=sBx; R(A+3)=R(A) }
    FORLOOP = 31,
    /// R(A)-=R(A+2); pc+=sBx
    FORPREP = 32,
    /// R(A+3), ... ,R(A+2+C) := R(A)(R(A+1), R(A+2))
    TFORLOOP = 33,

    // ── 表初始化指令 ──────────────────────────────────────────────
    /// R(A)[(C-1)*FPF+i] := R(A+i), 1 <= i <= B
    SETLIST = 34,

    // ── 栈管理指令 ────────────────────────────────────────────────
    /// close all variables in the stack up to (>=) R(A)
    CLOSE = 35,

    // ── 闭包创建指令 ──────────────────────────────────────────────
    /// R(A) := closure(KPROTO[Bx], R(A), ... ,R(A+n))
    CLOSURE = 36,

    // ── 可变参数指令 ──────────────────────────────────────────────
    /// R(A), R(A+1), ..., R(A+B-1) = vararg
    VARARG = 37,
}

/// 操作码数量
pub const NUM_OPCODES: i32 = 38;

impl OpCode {
    /// 从 u8 值构造 OpCode
    ///
    /// 如果值超出有效范围（≥ 38）则返回 `None`。
    pub fn from_u8(value: u8) -> Option<Self> {
        if value < NUM_OPCODES as u8 {
            // SAFETY: value is within valid discriminant range
            Some(unsafe { std::mem::transmute::<u8, OpCode>(value) })
        } else {
            None
        }
    }

    /// 检查是否为有效的操作码
    #[inline]
    pub fn is_valid(value: u8) -> bool {
        value < NUM_OPCODES as u8
    }
}

// =====================================================================
// 操作数类型枚举
// =====================================================================

/// 操作数类型掩码
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpArgMask {
    /// 参数未使用
    OpArgN,
    /// 参数被使用
    OpArgU,
    /// 寄存器或跳转偏移
    OpArgR,
    /// 常量或寄存器/常量（RK）
    OpArgK,
}

// =====================================================================
// 操作码分组枚举
// =====================================================================

/// 操作码功能分组
///
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpcodeGroup {
    Unknown,
    DataMove,
    Global,
    Upvalue,
    Table,
    Arithmetic,
    Unary,
    Branch,
    Comparison,
    Call,
    Loop,
    Closure,
    Vararg,
}

// =====================================================================
// 操作码元数据
// =====================================================================

/// 操作码元数据结构
///
#[derive(Debug, Clone)]
pub struct OpcodeMetadata {
    /// 操作码
    pub opcode: OpCode,
    /// 助记符名称
    pub name: &'static str,
    /// 指令格式
    pub mode: OpMode,
    /// B 操作数类型
    pub b_mode: OpArgMask,
    /// C 操作数类型
    pub c_mode: OpArgMask,
    /// 是否设置 A 寄存器
    pub sets_a: bool,
    /// 是否为测试类指令
    pub is_test: bool,
    /// 操作码分组
    pub group: OpcodeGroup,
    /// 是否可能触发元方法
    pub may_invoke_metamethod: bool,
}

// =====================================================================
// 元数据表（38 条，顺序必须与 OpCode 枚举完全一致）
// =====================================================================

/// 所有操作码的元数据表
///
/// 索引即为 `OpCode` 的 discriminant 值。
///
pub static OPCODE_METADATA: [OpcodeMetadata; 38] = [
    // MOVE
    OpcodeMetadata {
        opcode: OpCode::MOVE,
        name: "MOVE",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::DataMove,
        may_invoke_metamethod: false,
    },
    // LOADK
    OpcodeMetadata {
        opcode: OpCode::LOADK,
        name: "LOADK",
        mode: OpMode::IABx,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::DataMove,
        may_invoke_metamethod: false,
    },
    // LOADBOOL
    OpcodeMetadata {
        opcode: OpCode::LOADBOOL,
        name: "LOADBOOL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgU,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::DataMove,
        may_invoke_metamethod: false,
    },
    // LOADNIL
    OpcodeMetadata {
        opcode: OpCode::LOADNIL,
        name: "LOADNIL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::DataMove,
        may_invoke_metamethod: false,
    },
    // GETUPVAL
    OpcodeMetadata {
        opcode: OpCode::GETUPVAL,
        name: "GETUPVAL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Upvalue,
        may_invoke_metamethod: false,
    },
    // GETGLOBAL
    OpcodeMetadata {
        opcode: OpCode::GETGLOBAL,
        name: "GETGLOBAL",
        mode: OpMode::IABx,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Global,
        may_invoke_metamethod: false,
    },
    // GETTABLE
    OpcodeMetadata {
        opcode: OpCode::GETTABLE,
        name: "GETTABLE",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Table,
        may_invoke_metamethod: true,
    },
    // SETGLOBAL
    OpcodeMetadata {
        opcode: OpCode::SETGLOBAL,
        name: "SETGLOBAL",
        mode: OpMode::IABx,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgN,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Global,
        may_invoke_metamethod: false,
    },
    // SETUPVAL
    OpcodeMetadata {
        opcode: OpCode::SETUPVAL,
        name: "SETUPVAL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgN,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Upvalue,
        may_invoke_metamethod: false,
    },
    // SETTABLE
    OpcodeMetadata {
        opcode: OpCode::SETTABLE,
        name: "SETTABLE",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Table,
        may_invoke_metamethod: true,
    },
    // NEWTABLE
    OpcodeMetadata {
        opcode: OpCode::NEWTABLE,
        name: "NEWTABLE",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgU,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Table,
        may_invoke_metamethod: false,
    },
    // SELF
    OpcodeMetadata {
        opcode: OpCode::SELF,
        name: "SELF",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Table,
        may_invoke_metamethod: true,
    },
    // ADD
    OpcodeMetadata {
        opcode: OpCode::ADD,
        name: "ADD",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Arithmetic,
        may_invoke_metamethod: true,
    },
    // SUB
    OpcodeMetadata {
        opcode: OpCode::SUB,
        name: "SUB",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Arithmetic,
        may_invoke_metamethod: true,
    },
    // MUL
    OpcodeMetadata {
        opcode: OpCode::MUL,
        name: "MUL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Arithmetic,
        may_invoke_metamethod: true,
    },
    // DIV
    OpcodeMetadata {
        opcode: OpCode::DIV,
        name: "DIV",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Arithmetic,
        may_invoke_metamethod: true,
    },
    // MOD
    OpcodeMetadata {
        opcode: OpCode::MOD,
        name: "MOD",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Arithmetic,
        may_invoke_metamethod: true,
    },
    // POW
    OpcodeMetadata {
        opcode: OpCode::POW,
        name: "POW",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Arithmetic,
        may_invoke_metamethod: true,
    },
    // UNM
    OpcodeMetadata {
        opcode: OpCode::UNM,
        name: "UNM",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Unary,
        may_invoke_metamethod: true,
    },
    // NOT
    OpcodeMetadata {
        opcode: OpCode::NOT,
        name: "NOT",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Unary,
        may_invoke_metamethod: false,
    },
    // LEN
    OpcodeMetadata {
        opcode: OpCode::LEN,
        name: "LEN",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Unary,
        may_invoke_metamethod: true,
    },
    // CONCAT
    OpcodeMetadata {
        opcode: OpCode::CONCAT,
        name: "CONCAT",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgR,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Unary,
        may_invoke_metamethod: true,
    },
    // JMP
    OpcodeMetadata {
        opcode: OpCode::JMP,
        name: "JMP",
        mode: OpMode::IAsBx,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Branch,
        may_invoke_metamethod: false,
    },
    // EQ
    OpcodeMetadata {
        opcode: OpCode::EQ,
        name: "EQ",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: false,
        is_test: true,
        group: OpcodeGroup::Comparison,
        may_invoke_metamethod: true,
    },
    // LT
    OpcodeMetadata {
        opcode: OpCode::LT,
        name: "LT",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: false,
        is_test: true,
        group: OpcodeGroup::Comparison,
        may_invoke_metamethod: true,
    },
    // LE
    OpcodeMetadata {
        opcode: OpCode::LE,
        name: "LE",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgK,
        c_mode: OpArgMask::OpArgK,
        sets_a: false,
        is_test: true,
        group: OpcodeGroup::Comparison,
        may_invoke_metamethod: true,
    },
    // TEST
    OpcodeMetadata {
        opcode: OpCode::TEST,
        name: "TEST",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgU,
        sets_a: true,
        is_test: true,
        group: OpcodeGroup::Branch,
        may_invoke_metamethod: false,
    },
    // TESTSET
    OpcodeMetadata {
        opcode: OpCode::TESTSET,
        name: "TESTSET",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgU,
        sets_a: true,
        is_test: true,
        group: OpcodeGroup::Branch,
        may_invoke_metamethod: false,
    },
    // CALL
    OpcodeMetadata {
        opcode: OpCode::CALL,
        name: "CALL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgU,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Call,
        may_invoke_metamethod: true,
    },
    // TAILCALL
    OpcodeMetadata {
        opcode: OpCode::TAILCALL,
        name: "TAILCALL",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgU,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Call,
        may_invoke_metamethod: true,
    },
    // RETURN
    OpcodeMetadata {
        opcode: OpCode::RETURN,
        name: "RETURN",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgN,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Call,
        may_invoke_metamethod: false,
    },
    // FORLOOP
    OpcodeMetadata {
        opcode: OpCode::FORLOOP,
        name: "FORLOOP",
        mode: OpMode::IAsBx,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Loop,
        may_invoke_metamethod: false,
    },
    // FORPREP
    OpcodeMetadata {
        opcode: OpCode::FORPREP,
        name: "FORPREP",
        mode: OpMode::IAsBx,
        b_mode: OpArgMask::OpArgR,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Loop,
        may_invoke_metamethod: false,
    },
    // TFORLOOP
    OpcodeMetadata {
        opcode: OpCode::TFORLOOP,
        name: "TFORLOOP",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgN,
        c_mode: OpArgMask::OpArgU,
        sets_a: false,
        is_test: true,
        group: OpcodeGroup::Loop,
        may_invoke_metamethod: false,
    },
    // SETLIST
    OpcodeMetadata {
        opcode: OpCode::SETLIST,
        name: "SETLIST",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgU,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Table,
        may_invoke_metamethod: false,
    },
    // CLOSE
    OpcodeMetadata {
        opcode: OpCode::CLOSE,
        name: "CLOSE",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgN,
        c_mode: OpArgMask::OpArgN,
        sets_a: false,
        is_test: false,
        group: OpcodeGroup::Branch,
        may_invoke_metamethod: false,
    },
    // CLOSURE
    OpcodeMetadata {
        opcode: OpCode::CLOSURE,
        name: "CLOSURE",
        mode: OpMode::IABx,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Closure,
        may_invoke_metamethod: false,
    },
    // VARARG
    OpcodeMetadata {
        opcode: OpCode::VARARG,
        name: "VARARG",
        mode: OpMode::IABC,
        b_mode: OpArgMask::OpArgU,
        c_mode: OpArgMask::OpArgN,
        sets_a: true,
        is_test: false,
        group: OpcodeGroup::Vararg,
        may_invoke_metamethod: false,
    },
];

/// 未知操作码的元数据（越界查询时返回）
///
pub static UNKNOWN_OPCODE_METADATA: OpcodeMetadata = OpcodeMetadata {
    opcode: OpCode::MOVE, // 占位，实际不会被使用
    name: "UNKNOWN",
    mode: OpMode::IABC,
    b_mode: OpArgMask::OpArgN,
    c_mode: OpArgMask::OpArgN,
    sets_a: false,
    is_test: false,
    group: OpcodeGroup::Unknown,
    may_invoke_metamethod: false,
};

// =====================================================================
// 元数据查询函数
// =====================================================================

/// 获取操作码的元数据
///
#[inline]
pub fn opcode_metadata(op: OpCode) -> &'static OpcodeMetadata {
    let idx = op as usize;
    if idx < OPCODE_METADATA.len() {
        &OPCODE_METADATA[idx]
    } else {
        &UNKNOWN_OPCODE_METADATA
    }
}

/// 获取指令格式
#[inline]
pub fn get_op_mode(op: OpCode) -> OpMode {
    opcode_metadata(op).mode
}

/// 获取 B 参数类型
#[inline]
pub fn get_b_mode(op: OpCode) -> OpArgMask {
    opcode_metadata(op).b_mode
}

/// 获取 C 参数类型
#[inline]
pub fn get_c_mode(op: OpCode) -> OpArgMask {
    opcode_metadata(op).c_mode
}

/// 测试指令是否设置 A 寄存器
#[inline]
pub fn test_a_mode(op: OpCode) -> bool {
    opcode_metadata(op).sets_a
}

/// 测试指令是否为测试指令
#[inline]
pub fn test_t_mode(op: OpCode) -> bool {
    opcode_metadata(op).is_test
}

/// 获取操作码助记符名称
#[inline]
pub fn get_op_name(op: OpCode) -> &'static str {
    opcode_metadata(op).name
}

// =====================================================================
// 位操作辅助函数
// =====================================================================

/// 创建位掩码：从位置 p 开始，宽度 n 的全 1 位
#[inline]
const fn mask1(n: i32, p: i32) -> Instruction {
    (!((!0u32) << n)) << p
}

// =====================================================================
// 指令解码函数
// =====================================================================

/// 从指令中提取操作码
///
#[inline]
pub fn get_opcode(inst: Instruction) -> OpCode {
    let raw = ((inst >> POS_OP) & mask1(SIZE_OP, 0)) as u8;
    OpCode::from_u8(raw).unwrap_or(OpCode::MOVE)
}

/// 从指令中提取 A 参数
///
#[inline]
pub fn get_arg_a(inst: Instruction) -> i32 {
    ((inst >> POS_A) & mask1(SIZE_A, 0)) as i32
}

/// 从指令中提取 B 参数
///
#[inline]
pub fn get_arg_b(inst: Instruction) -> i32 {
    ((inst >> POS_B) & mask1(SIZE_B, 0)) as i32
}

/// 从指令中提取 C 参数
///
#[inline]
pub fn get_arg_c(inst: Instruction) -> i32 {
    ((inst >> POS_C) & mask1(SIZE_C, 0)) as i32
}

/// 从指令中提取 Bx 参数
///
#[inline]
pub fn get_arg_bx(inst: Instruction) -> i32 {
    ((inst >> POS_BX) & mask1(SIZE_BX, 0)) as i32
}

/// 从指令中提取 sBx 参数（有符号偏移）
///
#[inline]
pub fn get_arg_sbx(inst: Instruction) -> i32 {
    get_arg_bx(inst) - MAXARG_SBX
}

// =====================================================================
// 指令创建函数
// =====================================================================

/// 创建 iABC 格式指令
///
#[inline]
pub fn create_abc(op: OpCode, a: i32, b: i32, c: i32) -> Instruction {
    ((op as Instruction) << POS_OP)
        | ((a as Instruction) << POS_A)
        | ((b as Instruction) << POS_B)
        | ((c as Instruction) << POS_C)
}

/// 创建 iABx 格式指令
///
#[inline]
pub fn create_abx(op: OpCode, a: i32, bx: i32) -> Instruction {
    ((op as Instruction) << POS_OP)
        | ((a as Instruction) << POS_A)
        | ((bx as Instruction) << POS_BX)
}

/// 创建 iAsBx 格式指令
///
#[inline]
pub fn create_as_bx(op: OpCode, a: i32, sbx: i32) -> Instruction {
    create_abx(op, a, sbx + MAXARG_SBX)
}

// =====================================================================
// 指令字段修改函数
// =====================================================================

/// 设置指令的 A 参数
///
#[inline]
pub fn set_arg_a(inst: &mut Instruction, a: i32) {
    *inst =
        (*inst & !mask1(SIZE_A, POS_A)) | (((a as Instruction) << POS_A) & mask1(SIZE_A, POS_A));
}

/// 设置指令的 B 参数
///
#[inline]
pub fn set_arg_b(inst: &mut Instruction, b: i32) {
    *inst =
        (*inst & !mask1(SIZE_B, POS_B)) | (((b as Instruction) << POS_B) & mask1(SIZE_B, POS_B));
}

/// 设置指令的 C 参数
///
#[inline]
pub fn set_arg_c(inst: &mut Instruction, c: i32) {
    *inst =
        (*inst & !mask1(SIZE_C, POS_C)) | (((c as Instruction) << POS_C) & mask1(SIZE_C, POS_C));
}

/// 设置指令的 sBx 参数
///
#[inline]
pub fn set_arg_sbx(inst: &mut Instruction, sbx: i32) {
    let encoded = (sbx + MAXARG_SBX) as Instruction;
    *inst = (*inst & !mask1(SIZE_BX, POS_BX)) | ((encoded << POS_BX) & mask1(SIZE_BX, POS_BX));
}

// =====================================================================
// RK 寻址辅助函数
// =====================================================================

/// 判断操作数是否为常量（RK 编码）
///
#[inline]
pub fn is_k(x: i32) -> bool {
    (x & BITRK) != 0
}

/// 从 RK 操作数中提取常量索引
///
#[inline]
pub fn index_k(r: i32) -> i32 {
    r & !BITRK
}

/// 将常量索引编码为 RK 操作数
///
#[inline]
pub fn rk_ask(x: i32) -> i32 {
    x | BITRK
}

// =====================================================================
// 测试
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── 布局常量 ──────────────────────────────────────────────────

    #[test]
    fn test_layout_constants() {
        assert_eq!(SIZE_C, 9);
        assert_eq!(SIZE_B, 9);
        assert_eq!(SIZE_BX, 18);
        assert_eq!(SIZE_A, 8);
        assert_eq!(SIZE_OP, 6);

        assert_eq!(POS_OP, 0);
        assert_eq!(POS_A, 6);
        assert_eq!(POS_C, 14);
        assert_eq!(POS_B, 23);
        assert_eq!(POS_BX, 14);
    }

    #[test]
    fn test_max_values() {
        assert_eq!(MAXARG_BX, 262143);
        assert_eq!(MAXARG_SBX, 131071);
        assert_eq!(MAXARG_A, 255);
        assert_eq!(MAXARG_B, 511);
        assert_eq!(MAXARG_C, 511);
    }

    #[test]
    fn test_rk_constants() {
        assert_eq!(BITRK, 256);
        assert_eq!(MAXINDEXRK, 255);
        assert_eq!(NO_REG, 255);
    }

    // ── OpCode discriminant ────────────────────────────────────────

    #[test]
    fn test_opcode_discriminants() {
        assert_eq!(OpCode::MOVE as u8, 0);
        assert_eq!(OpCode::LOADK as u8, 1);
        assert_eq!(OpCode::LOADBOOL as u8, 2);
        assert_eq!(OpCode::LOADNIL as u8, 3);
        assert_eq!(OpCode::GETUPVAL as u8, 4);
        assert_eq!(OpCode::GETGLOBAL as u8, 5);
        assert_eq!(OpCode::GETTABLE as u8, 6);
        assert_eq!(OpCode::SETGLOBAL as u8, 7);
        assert_eq!(OpCode::SETUPVAL as u8, 8);
        assert_eq!(OpCode::SETTABLE as u8, 9);
        assert_eq!(OpCode::NEWTABLE as u8, 10);
        assert_eq!(OpCode::SELF as u8, 11);
        assert_eq!(OpCode::ADD as u8, 12);
        assert_eq!(OpCode::SUB as u8, 13);
        assert_eq!(OpCode::MUL as u8, 14);
        assert_eq!(OpCode::DIV as u8, 15);
        assert_eq!(OpCode::MOD as u8, 16);
        assert_eq!(OpCode::POW as u8, 17);
        assert_eq!(OpCode::UNM as u8, 18);
        assert_eq!(OpCode::NOT as u8, 19);
        assert_eq!(OpCode::LEN as u8, 20);
        assert_eq!(OpCode::CONCAT as u8, 21);
        assert_eq!(OpCode::JMP as u8, 22);
        assert_eq!(OpCode::EQ as u8, 23);
        assert_eq!(OpCode::LT as u8, 24);
        assert_eq!(OpCode::LE as u8, 25);
        assert_eq!(OpCode::TEST as u8, 26);
        assert_eq!(OpCode::TESTSET as u8, 27);
        assert_eq!(OpCode::CALL as u8, 28);
        assert_eq!(OpCode::TAILCALL as u8, 29);
        assert_eq!(OpCode::RETURN as u8, 30);
        assert_eq!(OpCode::FORLOOP as u8, 31);
        assert_eq!(OpCode::FORPREP as u8, 32);
        assert_eq!(OpCode::TFORLOOP as u8, 33);
        assert_eq!(OpCode::SETLIST as u8, 34);
        assert_eq!(OpCode::CLOSE as u8, 35);
        assert_eq!(OpCode::CLOSURE as u8, 36);
        assert_eq!(OpCode::VARARG as u8, 37);
    }

    #[test]
    fn test_opcode_count() {
        assert_eq!(NUM_OPCODES, 38);
    }

    #[test]
    fn test_opcode_from_u8() {
        assert_eq!(OpCode::from_u8(0), Some(OpCode::MOVE));
        assert_eq!(OpCode::from_u8(37), Some(OpCode::VARARG));
        assert_eq!(OpCode::from_u8(38), None);
        assert_eq!(OpCode::from_u8(255), None);
    }

    // ── 指令编解码 ────────────────────────────────────────────────

    #[test]
    fn test_create_and_decode_abc() {
        let inst = create_abc(OpCode::ADD, 1, 2, 3);
        assert_eq!(get_opcode(inst), OpCode::ADD);
        assert_eq!(get_arg_a(inst), 1);
        assert_eq!(get_arg_b(inst), 2);
        assert_eq!(get_arg_c(inst), 3);
    }

    #[test]
    fn test_create_and_decode_abx() {
        let inst = create_abx(OpCode::LOADK, 5, 100);
        assert_eq!(get_opcode(inst), OpCode::LOADK);
        assert_eq!(get_arg_a(inst), 5);
        assert_eq!(get_arg_bx(inst), 100);
    }

    #[test]
    fn test_create_and_decode_as_bx_positive() {
        let inst = create_as_bx(OpCode::JMP, 0, 42);
        assert_eq!(get_opcode(inst), OpCode::JMP);
        assert_eq!(get_arg_sbx(inst), 42);
    }

    #[test]
    fn test_create_and_decode_as_bx_negative() {
        let inst = create_as_bx(OpCode::JMP, 0, -10);
        assert_eq!(get_opcode(inst), OpCode::JMP);
        assert_eq!(get_arg_sbx(inst), -10);
    }

    #[test]
    fn test_create_abc_max_values() {
        let inst = create_abc(OpCode::VARARG, MAXARG_A, MAXARG_B, MAXARG_C);
        assert_eq!(get_opcode(inst), OpCode::VARARG);
        assert_eq!(get_arg_a(inst), MAXARG_A);
        assert_eq!(get_arg_b(inst), MAXARG_B);
        assert_eq!(get_arg_c(inst), MAXARG_C);
    }

    #[test]
    fn test_create_abx_max_values() {
        let inst = create_abx(OpCode::CLOSURE, MAXARG_A, MAXARG_BX);
        assert_eq!(get_opcode(inst), OpCode::CLOSURE);
        assert_eq!(get_arg_a(inst), MAXARG_A);
        assert_eq!(get_arg_bx(inst), MAXARG_BX);
    }

    // ── 指令字段修改 ──────────────────────────────────────────────

    #[test]
    fn test_set_arg_a() {
        let mut inst = create_abc(OpCode::MOVE, 1, 2, 3);
        set_arg_a(&mut inst, 99);
        assert_eq!(get_arg_a(inst), 99);
        assert_eq!(get_arg_b(inst), 2);
        assert_eq!(get_arg_c(inst), 3);
        assert_eq!(get_opcode(inst), OpCode::MOVE);
    }

    #[test]
    fn test_set_arg_b() {
        let mut inst = create_abc(OpCode::ADD, 1, 2, 3);
        set_arg_b(&mut inst, 77);
        assert_eq!(get_arg_b(inst), 77);
        assert_eq!(get_arg_a(inst), 1);
        assert_eq!(get_arg_c(inst), 3);
    }

    #[test]
    fn test_set_arg_c() {
        let mut inst = create_abc(OpCode::ADD, 1, 2, 3);
        set_arg_c(&mut inst, 88);
        assert_eq!(get_arg_c(inst), 88);
        assert_eq!(get_arg_a(inst), 1);
        assert_eq!(get_arg_b(inst), 2);
    }

    #[test]
    fn test_set_arg_sbx() {
        let mut inst = create_as_bx(OpCode::FORLOOP, 1, 0);
        set_arg_sbx(&mut inst, -5);
        assert_eq!(get_arg_sbx(inst), -5);
    }

    // ── RK 寻址 ───────────────────────────────────────────────────

    #[test]
    fn test_is_k() {
        assert!(!is_k(0));
        assert!(!is_k(255));
        assert!(is_k(256)); // BITRK
        assert!(is_k(300));
    }

    #[test]
    fn test_index_k() {
        assert_eq!(index_k(256), 0); // 0 | BITRK → index 0
        assert_eq!(index_k(257), 1); // 1 | BITRK → index 1
        assert_eq!(index_k(300), 44); // 44 | BITRK → index 44
    }

    #[test]
    fn test_rk_ask() {
        assert_eq!(rk_ask(0), 256); // index 0 → 0 | BITRK
        assert_eq!(rk_ask(5), 261); // index 5 → 5 | BITRK
        assert_eq!(rk_ask(255), 511); // index 255 → 255 | BITRK = 511 (max RK)
    }

    // ── 元数据表 ──────────────────────────────────────────────────

    #[test]
    fn test_metadata_table_size() {
        assert_eq!(OPCODE_METADATA.len(), 38);
    }

    #[test]
    fn test_metadata_table_enum_order() {
        // 元数据表的索引必须与 OpCode 的 discriminant 值完全一致
        for (index, meta) in OPCODE_METADATA.iter().enumerate() {
            assert_eq!(
                meta.opcode as usize, index,
                "Metadata index {index} has opcode {:?}",
                meta.opcode
            );
        }
    }

    #[test]
    fn test_metadata_all_names_present() {
        for meta in &OPCODE_METADATA {
            assert!(!meta.name.is_empty());
            assert!(!matches!(meta.group, OpcodeGroup::Unknown));
        }
    }

    #[test]
    fn test_opcode_metadata_query() {
        let meta = opcode_metadata(OpCode::MOVE);
        assert_eq!(meta.name, "MOVE");
        assert_eq!(meta.mode, OpMode::IABC);

        let meta = opcode_metadata(OpCode::LOADK);
        assert_eq!(meta.name, "LOADK");
        assert_eq!(meta.mode, OpMode::IABx);

        let meta = opcode_metadata(OpCode::JMP);
        assert_eq!(meta.name, "JMP");
        assert_eq!(meta.mode, OpMode::IAsBx);
    }

    #[test]
    fn test_get_op_mode() {
        assert_eq!(get_op_mode(OpCode::MOVE), OpMode::IABC);
        assert_eq!(get_op_mode(OpCode::LOADK), OpMode::IABx);
        assert_eq!(get_op_mode(OpCode::JMP), OpMode::IAsBx);
    }

    #[test]
    fn test_get_b_mode() {
        assert_eq!(get_b_mode(OpCode::ADD), OpArgMask::OpArgK);
        assert_eq!(get_b_mode(OpCode::MOVE), OpArgMask::OpArgR);
        assert_eq!(get_b_mode(OpCode::CLOSE), OpArgMask::OpArgN);
    }

    #[test]
    fn test_get_c_mode() {
        assert_eq!(get_c_mode(OpCode::ADD), OpArgMask::OpArgK);
        assert_eq!(get_c_mode(OpCode::LOADK), OpArgMask::OpArgN);
    }

    #[test]
    fn test_test_a_mode() {
        assert!(test_a_mode(OpCode::ADD));
        assert!(!test_a_mode(OpCode::SETTABLE));
        assert!(!test_a_mode(OpCode::EQ));
    }

    #[test]
    fn test_test_t_mode() {
        assert!(!test_t_mode(OpCode::ADD));
        assert!(test_t_mode(OpCode::EQ));
        assert!(test_t_mode(OpCode::LT));
        assert!(test_t_mode(OpCode::LE));
        assert!(test_t_mode(OpCode::TEST));
    }

    #[test]
    fn test_get_op_name() {
        assert_eq!(get_op_name(OpCode::MOVE), "MOVE");
        assert_eq!(get_op_name(OpCode::ADD), "ADD");
        assert_eq!(get_op_name(OpCode::VARARG), "VARARG");
    }

    // ── 所有 38 个 opcode 的 roundtrip ────────────────────────────

    #[test]
    fn test_all_opcodes_roundtrip() {
        for op in 0..38u8 {
            let opcode = OpCode::from_u8(op).unwrap();
            let mode = get_op_mode(opcode);
            match mode {
                OpMode::IABC => {
                    let inst = create_abc(opcode, 1, 2, 3);
                    assert_eq!(get_opcode(inst), opcode);
                    assert_eq!(get_arg_a(inst), 1);
                    assert_eq!(get_arg_b(inst), 2);
                    assert_eq!(get_arg_c(inst), 3);
                }
                OpMode::IABx => {
                    let inst = create_abx(opcode, 1, 100);
                    assert_eq!(get_opcode(inst), opcode);
                    assert_eq!(get_arg_a(inst), 1);
                    assert_eq!(get_arg_bx(inst), 100);
                }
                OpMode::IAsBx => {
                    let inst = create_as_bx(opcode, 0, 42);
                    assert_eq!(get_opcode(inst), opcode);
                    assert_eq!(get_arg_sbx(inst), 42);
                }
            }
        }
    }

    #[test]
    fn test_instruction_bit_isolation() {
        // 验证不同指令的字段不会互相干扰
        let inst1 = create_abc(OpCode::MOVE, 255, 511, 511);
        let inst2 = create_abc(OpCode::ADD, 0, 0, 0);
        assert_ne!(inst1, inst2);

        // MOVE 和 ADD 的 OP 字段不同
        assert_eq!(get_opcode(inst1), OpCode::MOVE);
        assert_eq!(get_opcode(inst2), OpCode::ADD);
    }

    // ── UNKNOWN 元数据 ─────────────────────────────────────────────

    #[test]
    fn test_unknown_opcode_metadata() {
        assert_eq!(UNKNOWN_OPCODE_METADATA.name, "UNKNOWN");
        assert_eq!(UNKNOWN_OPCODE_METADATA.group, OpcodeGroup::Unknown);
    }
}
