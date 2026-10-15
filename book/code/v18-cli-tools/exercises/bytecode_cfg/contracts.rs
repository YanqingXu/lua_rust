use crate::fixtures;
use crate::model::{graph, Leaders};
use std::fmt::Debug;

pub fn equal<T: PartialEq + Debug>(label: &str, actual: T, expected: T) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label}: expected {expected:?}, got {actual:?}"))
    }
}

// ANCHOR: cfg_leaders_contract
pub fn leaders_split_control_flow_and_validate_instruction_targets(
    discover: Leaders,
) -> Result<(), String> {
    for case in fixtures::matrix() {
        equal(case.name, discover(&case.proto)?, case.leaders)?;
        equal(case.name, graph(&case.proto, discover)?, case.graph.clone())?;
        // Full DOT, including every block's instruction range and every labeled edge.
        equal(
            case.name,
            lua_bytecode::cfg_dot(&case.proto)?,
            case.graph.dot(&case.proto)?,
        )?;
    }
    for (proto, expected_error) in fixtures::invalid() {
        equal(
            "leader validation",
            discover(&proto),
            Err(expected_error.clone()),
        )?;
        equal(
            "public CFG validation",
            lua_bytecode::cfg_dot(&proto),
            Err(expected_error),
        )?;
    }
    Ok(())
}
// ANCHOR_END: cfg_leaders_contract
