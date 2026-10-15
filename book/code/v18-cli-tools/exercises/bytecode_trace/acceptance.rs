mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "实现 starter::dispatch，再运行 --ignored"]
fn records_current_registers_before_dispatch() {
    contracts::records_current_registers_before_dispatch(starter::dispatch);
}
