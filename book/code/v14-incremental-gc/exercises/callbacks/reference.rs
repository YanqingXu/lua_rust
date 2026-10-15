mod contracts;
mod faults;
mod model;
mod real;
mod solution;

use model::{CallbackVm, Design, EventError};

#[test]
fn transferred_batch_is_invisible_to_reentry() {
    contracts::transfers_batch(solution::take_batch);
}

#[test]
fn whole_batch_roots_protect_siblings() {
    contracts::protects_sibling_events(solution::pin_batch);
}

#[test]
fn callback_error_drains_and_restores_roots() {
    contracts::drains_after_error(solution::drain_batch);
}

#[test]
fn faulty_clone_replays_each_event_twice_with_one_bounded_reentry() {
    let mut vm = CallbackVm::reentrant_pair(Design {
        take: faults::clone_batch,
        ..contracts::healthy()
    });
    assert_eq!(vm.drain_pending(), Ok(()));
    assert_eq!(vm.calls, [1, 1, 2, 2]);
    assert_eq!(vm.pending.len(), 2);
    assert_eq!(vm.roots(), [900]);
}

#[test]
fn faulty_current_only_roots_lose_the_owned_sibling() {
    let mut vm = CallbackVm::reentrant_pair(Design {
        pin: faults::current_only,
        ..contracts::healthy()
    });
    assert_eq!(vm.drain_pending(), Err(EventError::Stale(2)));
    assert_eq!(vm.calls, [1]);
    assert!(!vm.is_live(2) && !vm.is_live(102));
    assert_eq!(vm.roots(), [900]);
}

#[test]
fn faulty_question_mark_skips_siblings_and_leaks_batch_roots() {
    let mut vm = CallbackVm::failing_trio(Design {
        drain: faults::early_return,
        ..contracts::healthy()
    });
    assert_eq!(vm.drain_pending(), Err(EventError::Callback(1)));
    assert_eq!(vm.calls, [1]);
    assert_eq!(vm.root_len(), 7);
    vm.collect();
    assert!(vm.is_live(2) && vm.is_live(3));
}

#[test]
fn real_heap_owned_vec_requires_explicit_roots() {
    assert_eq!(real::heap_owned_batch_accessible(false), (false, false));
    assert_eq!(real::heap_owned_batch_accessible(true), (true, true));
}

#[test]
fn real_vm_reentrant_finalizer_keeps_sibling_alive_and_runs_once() {
    assert_eq!(real::vm_reentrant_pair(), lua_core::Value::Number(123.0));
}

#[test]
fn real_vm_callback_error_still_executes_the_sibling() {
    assert_eq!(
        real::vm_error_then_sibling(),
        (lua_vm::VmErrorKind::Type, lua_core::Value::Boolean(true))
    );
}
