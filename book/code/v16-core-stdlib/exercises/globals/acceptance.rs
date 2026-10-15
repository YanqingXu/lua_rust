mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "TODO：globals/starter.rs::install，显式验收"]
fn single_environment_is_shared_by_globals_and_g() {
    contracts::single_environment(starter::install);
}
