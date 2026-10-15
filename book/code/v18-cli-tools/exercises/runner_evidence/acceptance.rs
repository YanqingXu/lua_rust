mod contracts;
pub mod model;
mod starter;
pub mod storage;
#[path = "../support/temp.rs"]
pub mod temp;

#[test]
#[ignore = "实现 starter::archive，再运行 --ignored"]
fn archives_observed_bytes_and_same_run_metadata() {
    contracts::archives_observed_bytes_and_same_run_metadata(starter::archive).unwrap();
}
