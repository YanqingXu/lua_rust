use super::model::Lifecycle;

// ANCHOR: timeout_starter
pub fn stop(_child: &mut dyn Lifecycle) -> Result<(), String> {
    // TODO: the deadline is not cleanup; attempt kill and always attempt wait.
    Ok(())
}
// ANCHOR_END: timeout_starter
