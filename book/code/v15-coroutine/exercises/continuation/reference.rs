mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use model::{ContinuationVm, Design, Outcome, Status, TurnVm, Value};

#[test]
fn suspended_frames_belong_to_each_coroutine() {
    contracts::per_coroutine_storage(solution::save_frame);
}

#[test]
fn resume_does_not_restart_entry_side_effects() {
    contracts::resumes_after_yield(solution::resume_pc);
}

#[test]
fn nested_owner_turn_restores_parent_and_channels() {
    contracts::restores_parent_turn(solution::run_child);
}

#[test]
fn faulty_shared_scratch_contaminates_another_coroutines_local() {
    let mut vm = ContinuationVm::new(
        Design {
            save: faults::shared_scratch,
            ..contracts::healthy()
        },
        &[(1, 10), (2, 100)],
    );
    vm.resume(1, 0).unwrap();
    vm.resume(2, 0).unwrap();
    assert_eq!(
        vm.resume(1, 1),
        Ok(Outcome::Yield(vec![Value::Number(101), Value::Nil]))
    );
    assert_eq!(
        vm.resume(2, 2),
        Ok(Outcome::Return(vec![Value::Number(103), Value::Nil]))
    );
    assert_eq!(vm.resume(1, 3), Err(model::ResumeError::LostFrame(1)));
}

#[test]
fn faulty_restart_repeats_the_entry_side_effect() {
    let mut vm = ContinuationVm::new(
        Design {
            pc: faults::restart_entry,
            ..contracts::healthy()
        },
        &[(1, 10)],
    );
    for input in [0, 5, 7] {
        assert_eq!(
            vm.resume(1, input),
            Ok(Outcome::Yield(vec![Value::Number(10), Value::Nil]))
        );
    }
    assert_eq!(vm.starts[&1], 3);
    assert_eq!(vm.coroutines[&1].status, Status::Suspended);
}

#[test]
fn faulty_shared_channels_mix_parent_and_child_payloads() {
    let mut vm = TurnVm::new();
    let child = faults::share_turn_channels(&mut vm);
    assert_eq!(vm.observed_parent, Some(Status::Running));
    assert!(!vm.child_saw_empty_channels);
    assert_eq!(
        child.values,
        [
            Value::Number(900),
            Value::Number(7),
            Value::Nil,
            Value::Number(9)
        ]
    );
    assert_eq!(child.frames, ["outer-frame", "child-frame"]);
    assert_eq!(vm.yield_values, Some(child.values));
}

#[test]
fn real_vm_interleaves_two_independent_coroutine_stacks() {
    assert_eq!(
        real::interleaved_locals(),
        vec![
            lua_core::Value::Boolean(true),
            lua_core::Value::Number(10.0),
            lua_core::Value::Number(100.0),
            lua_core::Value::Number(11.0),
            lua_core::Value::Number(102.0),
            lua_core::Value::Number(14.0),
            lua_core::Value::Number(106.0),
            lua_core::Value::Boolean(true)
        ]
    );
}

#[test]
fn real_vm_restores_nested_callers_without_replaying_entry_effects() {
    use lua_core::Value::{Boolean, Nil, Number};
    assert_eq!(
        real::nested_calls_and_multiple_yields(),
        [
            vec![Boolean(true), Number(10.0), Nil],
            vec![Boolean(true), Number(21.0), Nil],
            vec![Boolean(true), Number(32.0), Number(1.0), Nil]
        ]
    );
}

#[test]
fn real_vm_nested_statuses_and_nil_values_follow_the_resume_direction() {
    use lua_core::Value::{Boolean, Nil, Number};
    assert_eq!(
        real::nested_statuses_and_value_direction(),
        (
            vec![
                Boolean(true),
                Boolean(true),
                Boolean(true),
                Boolean(true),
                Nil,
                Boolean(true),
                Boolean(true),
                Number(7.0),
                Nil,
                Number(9.0),
                Boolean(true)
            ],
            Boolean(true)
        )
    );
}
