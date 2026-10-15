mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "TODO: map Result to exit 0/1/2 while preserving stdout and stderr roles"]
fn result_boundary_preserves_streams_and_maps_exit_codes() {
    contracts::result_boundary(starter::finish);
}
