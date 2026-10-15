mod contracts;
pub mod model;
mod starter;

#[test]
#[ignore = "实现 starter::decide，再运行 --ignored"]
fn structured_errors_choose_the_next_repl_action() {
    contracts::structured_errors_choose_the_next_repl_action(starter::decide);
}
