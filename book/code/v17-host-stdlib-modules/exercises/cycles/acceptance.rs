mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "TODO：cycles/starter.rs::run_loader，显式验收"]
fn loading_sentinel_is_visible_before_recursive_loader_call() {
    contracts::sentinel_precedes_recursive_loader(starter::run_loader);
}
