use crate::model::{has_fallthrough, is_jump, target};
use lua_core::Proto;
use std::collections::BTreeSet;

// ANCHOR: cfg_leaders_solution
pub fn leaders(proto: &Proto) -> Result<Vec<usize>, String> {
    if proto.code.is_empty() {
        return Ok(Vec::new());
    }
    let mut leaders = BTreeSet::from([0]);
    for (pc, instruction) in proto.code.iter().copied().enumerate() {
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        if is_jump(opcode) {
            leaders.insert(target(pc, instruction.sbx(), proto.code.len())?);
        }
        if (is_jump(opcode) || !has_fallthrough(opcode)) && pc + 1 < proto.code.len() {
            leaders.insert(pc + 1);
        }
    }
    Ok(leaders.into_iter().collect())
}
// ANCHOR_END: cfg_leaders_solution
