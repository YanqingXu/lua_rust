//! Auditable mapping between Lua 5.1's public VM vocabulary and the teaching ISA.
//!
//! v1.0 executes source-built `Proto` values. It deliberately does not claim
//! binary-chunk compatibility: serialization and verification start in v1.1.

use lua_core::Opcode;
use std::fmt::Write as _;

/// The 38 opcodes defined by Lua 5.1 (`lopcodes.h`), in their canonical order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Lua51Opcode {
    Move,
    LoadK,
    LoadBool,
    LoadNil,
    GetUpval,
    GetGlobal,
    GetTable,
    SetGlobal,
    SetUpval,
    SetTable,
    NewTable,
    SelfOp,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Unm,
    Not,
    Len,
    Concat,
    Jmp,
    Eq,
    Lt,
    Le,
    Test,
    TestSet,
    Call,
    TailCall,
    Return,
    ForLoop,
    ForPrep,
    TForLoop,
    SetList,
    Close,
    Closure,
    Vararg,
}

pub const LUA51_OPCODES: [Lua51Opcode; 38] = [
    Lua51Opcode::Move,
    Lua51Opcode::LoadK,
    Lua51Opcode::LoadBool,
    Lua51Opcode::LoadNil,
    Lua51Opcode::GetUpval,
    Lua51Opcode::GetGlobal,
    Lua51Opcode::GetTable,
    Lua51Opcode::SetGlobal,
    Lua51Opcode::SetUpval,
    Lua51Opcode::SetTable,
    Lua51Opcode::NewTable,
    Lua51Opcode::SelfOp,
    Lua51Opcode::Add,
    Lua51Opcode::Sub,
    Lua51Opcode::Mul,
    Lua51Opcode::Div,
    Lua51Opcode::Mod,
    Lua51Opcode::Pow,
    Lua51Opcode::Unm,
    Lua51Opcode::Not,
    Lua51Opcode::Len,
    Lua51Opcode::Concat,
    Lua51Opcode::Jmp,
    Lua51Opcode::Eq,
    Lua51Opcode::Lt,
    Lua51Opcode::Le,
    Lua51Opcode::Test,
    Lua51Opcode::TestSet,
    Lua51Opcode::Call,
    Lua51Opcode::TailCall,
    Lua51Opcode::Return,
    Lua51Opcode::ForLoop,
    Lua51Opcode::ForPrep,
    Lua51Opcode::TForLoop,
    Lua51Opcode::SetList,
    Lua51Opcode::Close,
    Lua51Opcode::Closure,
    Lua51Opcode::Vararg,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpcodeAudit {
    pub lua51: Lua51Opcode,
    /// True when one teaching instruction has the same semantic role.
    pub direct: bool,
    /// One or more internal instructions used to realize the Lua 5.1 behavior.
    pub teaching: &'static [Opcode],
    pub note: &'static str,
}

macro_rules! audit {
    ($lua:ident => [$($internal:ident),+], $direct:expr, $note:literal) => {
        OpcodeAudit {
            lua51: Lua51Opcode::$lua,
            direct: $direct,
            teaching: &[$(Opcode::$internal),+],
            note: $note,
        }
    };
}

/// Complete semantic coverage table. Composite rows are intentional and must
/// not be presented as Lua 5.1 binary encoding compatibility.
pub const LUA51_OPCODE_AUDIT: [OpcodeAudit; 38] = [
    audit!(Move => [Move], true, "register copy"),
    audit!(LoadK => [LoadK], true, "constant load"),
    audit!(LoadBool => [LoadK], false, "booleans are regular constants"),
    audit!(LoadNil => [LoadK], false, "nil is a regular constant"),
    audit!(GetUpval => [GetUpval], true, "read captured cell"),
    audit!(GetGlobal => [GetGlobal], true, "read environment table"),
    audit!(GetTable => [GetTable], true, "indexed read with metamethod fallback"),
    audit!(SetGlobal => [SetGlobal], true, "write environment table"),
    audit!(SetUpval => [SetUpval], true, "write captured cell"),
    audit!(SetTable => [SetTable], true, "indexed write with metamethod fallback"),
    audit!(NewTable => [NewTable], true, "allocate table"),
    audit!(SelfOp => [GetTable, Move], false, "method lookup and self placement are explicit"),
    audit!(Add => [Add], true, "addition"),
    audit!(Sub => [Sub], true, "subtraction"),
    audit!(Mul => [Mul], true, "multiplication"),
    audit!(Div => [Div], true, "division"),
    audit!(Mod => [Mod], true, "Lua floor modulo"),
    audit!(Pow => [Pow], true, "exponentiation"),
    audit!(Unm => [Neg], true, "unary minus"),
    audit!(Not => [Not], true, "Lua truth-value negation"),
    audit!(Len => [Len], true, "length operator"),
    audit!(Concat => [Concat], true, "range concatenation semantic"),
    audit!(Jmp => [Jump], true, "relative branch"),
    audit!(Eq => [Eq, Ne], false, "positive and inverted comparisons have distinct teaching ops"),
    audit!(Lt => [Lt, Gt], false, "operand reversal is explicit"),
    audit!(Le => [Le, Ge], false, "operand reversal is explicit"),
    audit!(Test => [JumpIfFalse, JumpIfTrue], false, "truth branches are explicit"),
    audit!(TestSet => [JumpIfFalse, JumpIfTrue, Move], false, "short-circuit copy is explicit"),
    audit!(Call => [Call], true, "open or fixed result call"),
    audit!(TailCall => [TailCall], true, "frame-reusing call"),
    audit!(Return => [Return], true, "open or fixed result return"),
    audit!(ForLoop => [ForLoop], true, "numeric loop step and test"),
    audit!(ForPrep => [ForPrep], true, "numeric loop preparation"),
    audit!(TForLoop => [Call, JumpIfNil, Move], false, "iterator call/test/copy are explicit"),
    audit!(SetList => [SetList, SetListOpen], false, "fixed and open list writes are explicit"),
    audit!(Close => [Close], true, "close open upvalues"),
    audit!(Closure => [Closure], true, "instantiate nested proto"),
    audit!(Vararg => [Vararg], true, "open or fixed vararg copy"),
];

pub fn opcode_audit_json() -> String {
    let mut output = String::from(
        "{\"schema\":\"lua-rust-lua51-opcode-audit/v1\",\"binary_chunk_compatible\":false,\"opcodes\":[",
    );
    for (index, row) in LUA51_OPCODE_AUDIT.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let teaching = row
            .teaching
            .iter()
            .map(|opcode| format!("\"{opcode:?}\""))
            .collect::<Vec<_>>()
            .join(",");
        let _ = write!(
            output,
            "{{\"ordinal\":{},\"lua51\":\"{:?}\",\"direct\":{},\"teaching\":[{}],\"note\":\"{}\"}}",
            row.lua51 as u8, row.lua51, row.direct, teaching, row.note
        );
    }
    output.push_str("]}\n");
    output
}

#[cfg(test)]
mod tests {
    use super::{opcode_audit_json, LUA51_OPCODES, LUA51_OPCODE_AUDIT};
    use std::collections::BTreeSet;

    #[test]
    fn canonical_lua51_opcode_set_has_exactly_38_ordered_members() {
        assert_eq!(LUA51_OPCODES.len(), 38);
        for (ordinal, opcode) in LUA51_OPCODES.iter().copied().enumerate() {
            assert_eq!(opcode as usize, ordinal);
        }
    }

    #[test]
    fn every_lua51_opcode_has_one_nonempty_audit_row() {
        assert_eq!(LUA51_OPCODE_AUDIT.len(), LUA51_OPCODES.len());
        let mut seen = BTreeSet::new();
        for row in LUA51_OPCODE_AUDIT {
            assert!(seen.insert(row.lua51 as u8));
            assert!(!row.teaching.is_empty());
            assert!(!row.note.is_empty());
        }
        assert_eq!(seen.len(), 38);
    }

    #[test]
    fn machine_readable_audit_denies_binary_compatibility_claim() {
        let json = opcode_audit_json();
        assert!(json.contains("\"schema\":\"lua-rust-lua51-opcode-audit/v1\""));
        assert!(json.contains("\"binary_chunk_compatible\":false"));
        assert!(json.contains("\"lua51\":\"Vararg\""));
    }
}
