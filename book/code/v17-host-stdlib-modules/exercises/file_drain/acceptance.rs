mod contracts;
pub mod model;
mod starter;
pub mod support;

#[test]
#[ignore = "实现 starter::shutdown，再运行 --ignored"]
fn drain_before_heap_shutdown() {
    contracts::drain_before_heap_shutdown(starter::shutdown);
}
