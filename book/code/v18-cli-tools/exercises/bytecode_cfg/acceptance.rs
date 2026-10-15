mod contracts;
mod fixtures;
pub mod model;
mod starter;

#[test]
#[ignore = "TODO: discover sorted validated leaders from bytecode control flow"]
fn leaders_split_control_flow_and_validate_instruction_targets() {
    contracts::leaders_split_control_flow_and_validate_instruction_targets(starter::leaders)
        .unwrap();
}
