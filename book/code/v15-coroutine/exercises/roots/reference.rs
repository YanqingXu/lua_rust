mod contracts;
mod faults;
mod model;
mod real;
mod solution;

#[test]
fn suspended_values_are_mirrored_before_collection() {
    contracts::suspended_mirror_keeps_every_execution_value(solution::mirror_roots);
}

#[test]
fn unreachable_continuations_are_not_permanent_roots() {
    contracts::direct_roots_do_not_make_host_states_permanent(solution::extra_direct_roots);
}

#[test]
fn faulty_entry_only_mirror_leaves_owned_frames_with_stale_values() {
    let mut vm = model::VmModel::suspended();
    vm.collect(faults::entry_only, solution::extra_direct_roots, true);
    assert!(vm.is_live(model::THREAD));
    assert_eq!(vm.states.len(), 1);
    assert!(vm.resume_check().is_err());
    assert!(!vm.is_live(7) && !vm.is_live(8));
}

#[test]
fn faulty_scheduler_and_upvalue_roots_keep_an_unreachable_state() {
    let mut vm = model::VmModel::suspended();
    vm.collect(
        solution::mirror_roots,
        faults::root_scheduler_and_all_upvalues,
        false,
    );
    assert_eq!(vm.states.len(), 1);
    assert_eq!(vm.resume_check(), Ok(()));
}

#[test]
fn rooting_only_all_open_upvalues_still_leaks_the_cycle() {
    let mut vm = model::VmModel::suspended();
    vm.collect(solution::mirror_roots, faults::root_all_upvalues, false);
    assert!(vm.is_live(model::THREAD) && vm.is_live(6));
    assert_eq!(vm.states.len(), 1);
}

#[test]
fn mirror_copies_current_values_without_moving_execution_slots() {
    let mut vm = model::VmModel::suspended();
    let slot = std::rc::Rc::clone(&vm.states[&model::THREAD].frames[0].registers[0]);
    vm.collect(solution::mirror_roots, solution::extra_direct_roots, true);
    assert_eq!(vm.mirror, [4, 8, 7, 9]);
    *slot.borrow_mut() = 8;
    assert_eq!(vm.mirror, [4, 8, 7, 9], "旧 Value 镜像不随 slot 自动更新");
    vm.collect(solution::mirror_roots, solution::extra_direct_roots, true);
    assert_eq!(vm.mirror, [4, 8, 8, 9]);
    assert!(!vm.is_live(7));
    assert!(std::rc::Rc::ptr_eq(
        &slot,
        &vm.states[&model::THREAD].frames[0].registers[0]
    ));
}

#[test]
fn real_heap_mirrors_preserve_values_and_handle_removal_reclaims_the_graph() {
    let missing = real::heap_mirror_then_release(false);
    assert!(missing[0].thread && missing[0].prefix);
    assert_eq!(missing[0].graph, [false; 8]);
    let correct = real::heap_mirror_then_release(true);
    assert!(correct[0].thread && correct[0].prefix);
    assert_eq!(correct[0].graph, [true; 8]);
    for released in [&missing[1], &correct[1]] {
        assert!(!released.thread);
        assert_eq!(released.graph, [false; 8]);
        assert!(released.prefix);
    }
}

#[test]
fn real_vm_entry_register_capture_and_suspended_varargs_survive_gc() {
    assert_eq!(
        real::vm_keeps_entry_and_suspended_values(),
        lua_core::Value::Boolean(true)
    );
}

#[test]
fn real_vm_unreachable_cycle_disappears_from_weak_slots_and_scheduler() {
    assert_eq!(
        real::vm_unreachable_cycle_is_pruned(),
        (0, true, lua_core::Value::Boolean(true))
    );
}
