mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "实现 starter::submit，再运行 --ignored"]
fn source_lifecycle_recovers_after_errors() {
    contracts::source_lifecycle_recovers_after_errors(starter::submit);
}
