use crate::model::{has_fallthrough, is_jump, target};
use lua_core::Proto;
use std::collections::BTreeSet;

pub fn entry_and_targets(proto: &Proto) -> Result<Vec<usize>, String> {
    if proto.code.is_empty() {
        return Ok(vec![]);
    }
    let mut starts = BTreeSet::from([0]);
    for (pc, instruction) in proto.code.iter().copied().enumerate() {
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        if is_jump(opcode) {
            starts.insert(target(pc, instruction.sbx(), proto.code.len())?);
        }
    }
    Ok(starts.into_iter().collect())
}

pub fn pc_relative(proto: &Proto) -> Result<Vec<usize>, String> {
    if proto.code.is_empty() {
        return Ok(vec![]);
    }
    let mut starts = BTreeSet::from([0]);
    for (pc, instruction) in proto.code.iter().copied().enumerate() {
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        if is_jump(opcode) {
            let wrong = pc as i64 + i64::from(instruction.sbx());
            if wrong < 0 || wrong >= proto.code.len() as i64 {
                return Err(format!("jump at pc {pc} targets {wrong} outside code"));
            }
            starts.insert(wrong as usize);
        }
        if (is_jump(opcode) || !has_fallthrough(opcode)) && pc + 1 < proto.code.len() {
            starts.insert(pc + 1);
        }
    }
    Ok(starts.into_iter().collect())
}

pub fn source_lines(proto: &Proto) -> Result<Vec<usize>, String> {
    let mut starts = BTreeSet::new();
    for pc in 0..proto.code.len() {
        if pc == 0 || proto.lines.get(pc) != proto.lines.get(pc - 1) {
            starts.insert(pc);
        }
    }
    Ok(starts.into_iter().collect())
}
