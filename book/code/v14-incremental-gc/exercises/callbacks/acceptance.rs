mod contracts;
mod model;
mod solution;
mod starter;

#[test]
#[ignore = "TODO 1：只修改 callbacks/starter.rs::take_batch，显式验收"]
fn transferred_batch_is_invisible_to_reentry() {
    contracts::transfers_batch(starter::take_batch);
}

#[test]
#[ignore = "TODO 2：只修改 callbacks/starter.rs::pin_batch，显式验收"]
fn whole_batch_roots_protect_siblings() {
    contracts::protects_sibling_events(starter::pin_batch);
}

#[test]
#[ignore = "TODO 3：只修改 callbacks/starter.rs::drain_batch，显式验收"]
fn callback_error_drains_and_restores_roots() {
    contracts::drains_after_error(starter::drain_batch);
}
