mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "TODO: kill the exact owned child and always attempt wait"]
fn timeout_cleanup_always_attempts_wait_and_releases_exact_owner() {
    contracts::timeout_cleanup_always_attempts_wait_and_releases_exact_owner(starter::stop)
        .unwrap();
}
