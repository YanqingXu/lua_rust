mod contracts;
mod expected;
pub mod faults;
pub mod model;
pub mod real;
mod starter;

#[test]
#[ignore = "实现 catalog/starter.rs 的唯一 install TODO 后显式验收"]
fn catalog_installs_the_independent_public_contract() {
    contracts::complete_catalog(starter::install);
}
