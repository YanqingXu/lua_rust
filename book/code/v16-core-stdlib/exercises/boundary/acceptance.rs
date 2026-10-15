mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "TODO: capture Result, restore depth, then return it"]
fn protected_boundary_restores_depth_on_every_result() {
    contracts::restores_depth_before_returning(starter::invoke_boundary);
}
