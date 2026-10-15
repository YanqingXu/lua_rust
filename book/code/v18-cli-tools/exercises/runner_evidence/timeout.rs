//! 组合真实 timeout adapter；使用其健康 stop，不调用任何 learner hook。
#[path = "../runner_timeout/model.rs"]
pub mod model;
#[path = "../runner_timeout/real.rs"]
pub mod real;
#[path = "../runner_timeout/solution.rs"]
pub mod solution;
