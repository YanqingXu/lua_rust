pub mod chunks;
mod contracts;
mod starter;

#[test]
#[ignore = "TODO: reuse the caller's Session for every submission"]
fn submissions_preserve_globals_tables_closures_and_owner_edges() {
    contracts::submissions_preserve_globals_tables_closures_and_owner_edges(starter::submit)
        .unwrap();
}
