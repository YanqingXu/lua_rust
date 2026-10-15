mod chunks;
mod contracts;
mod faults;
#[path = "../support/process.rs"]
pub mod process;
mod real;
mod solution;
#[path = "../support/temp.rs"]
pub mod temp;

use lua_core::Value;
use std::path::Path;

#[test]
fn submissions_preserve_globals_tables_closures_and_owner_edges() {
    contracts::submissions_preserve_globals_tables_closures_and_owner_edges(solution::submit)
        .unwrap();
}

#[test]
fn fresh_session_per_submission_discards_previous_state() {
    let [setup, observed] = chunks::observe_pair(faults::fresh_session).unwrap();
    assert_eq!(setup, chunks::setup_values());
    assert_eq!(observed, vec![Value::Nil; 7]);
}

#[test]
fn copied_host_values_need_vm_roots_and_are_stale_after_release() {
    real::copied_host_handle_is_not_a_root().unwrap();
}

#[test]
fn real_cli_multiple_e_actions_then_script_share_session() {
    real::cli_multiple_actions_then_script(Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap();
}

#[test]
fn real_cli_e_action_then_interactive_inputs_share_session() {
    real::cli_actions_then_interactive_inputs(Path::new(env!("CARGO_BIN_EXE_lua_app"))).unwrap();
}
