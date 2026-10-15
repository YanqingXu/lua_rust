use super::model::Lifecycle;

// ANCHOR: timeout_solution
pub fn stop(child: &mut dyn Lifecycle) -> Result<(), String> {
    let killed = child.kill();
    child
        .wait()
        .map_err(|error| format!("wait after kill ({killed:?}): {error}"))?;
    Ok(())
}
// ANCHOR_END: timeout_solution
