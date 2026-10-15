use super::model::Lifecycle;

pub fn return_without_stopping(_child: &mut dyn Lifecycle) -> Result<(), String> {
    Ok(())
}

pub fn kill_without_waiting(child: &mut dyn Lifecycle) -> Result<(), String> {
    let _ = child.kill();
    Ok(())
}
