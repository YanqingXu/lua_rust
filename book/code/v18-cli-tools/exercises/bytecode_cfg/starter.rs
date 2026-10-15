use lua_core::Proto;

// ANCHOR: cfg_leaders_starter
pub fn leaders(proto: &Proto) -> Result<Vec<usize>, String> {
    // TODO: discover validated control-flow boundaries in this Proto.
    Ok(if proto.code.is_empty() {
        vec![]
    } else {
        vec![0]
    })
}
// ANCHOR_END: cfg_leaders_starter
