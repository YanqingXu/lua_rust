use lua_core::allocator::{AllocationSite, ManagedAllocationError};
use lua_vm::{LuaState, Runtime, StateResolveError};

#[test]
fn state_arena_failure_is_transactional_and_runtime_remains_usable() {
    let mut runtime = Runtime::new();
    let before = runtime.allocator_stats();
    runtime.inject_allocator_failure_after(AllocationSite::StateArenaSlot, 0);

    let mut callback_ran = false;
    {
        let mut parts = runtime.parts_mut().expect("Runtime parts are available");
        let (main, _, _) = parts.split_mut();
        let result = main.with_pending_coroutine_state(LuaState::new(), |_, _| {
            callback_ran = true;
        });
        assert!(matches!(
            result,
            Err(StateResolveError::Allocation(
                ManagedAllocationError::Injected {
                    site: AllocationSite::StateArenaSlot,
                    ..
                }
            ))
        ));
        assert_eq!(main.temporary_state_root_count(), 0);
    }
    assert!(!callback_ran);
    assert_eq!(runtime.live_coroutine_state_count(), 0);
    assert_eq!(runtime.temporary_state_root_count(), 0);
    assert_eq!(runtime.allocator_stats(), before);

    {
        let mut parts = runtime
            .parts_mut()
            .expect("one-shot failure leaves Runtime borrowable");
        let (main, _, _) = parts.split_mut();
        main.with_pending_coroutine_state(LuaState::new(), |_, publisher| {
            assert_eq!(publisher.temporary_state_root_count(), 1);
        })
        .expect("retry enters a temporary state root");
        assert_eq!(main.temporary_state_root_count(), 0);
    }
    assert_eq!(runtime.live_coroutine_state_count(), 0);
    assert_eq!(
        runtime.allocator_stats().live_bytes,
        before.live_bytes,
        "rolled-back child state must release its managed payload"
    );

    let report = runtime
        .close()
        .expect("Runtime closes after injected failure");
    assert_eq!(report.remaining_coroutine_states, 0);
    assert_eq!(report.remaining_temporary_state_roots, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.remaining_objects, 0);
}

#[test]
fn one_thousand_pending_state_lifetimes_close_at_zero() {
    let mut runtime = Runtime::new();
    let baseline_live = runtime.allocator_stats().live_bytes;

    for _cycle in 0..1_000 {
        let mut parts = runtime.parts_mut().expect("Runtime parts are available");
        let (main, _, _) = parts.split_mut();
        main.with_pending_coroutine_state(LuaState::new(), |_, publisher| {
            assert_eq!(publisher.temporary_state_root_count(), 1);
        })
        .expect("pending state is rolled back after the callback");
        assert_eq!(main.temporary_state_root_count(), 0);
    }

    assert_eq!(runtime.live_coroutine_state_count(), 0);
    assert_eq!(runtime.temporary_state_root_count(), 0);
    assert_eq!(runtime.allocator_stats().live_bytes, baseline_live);

    let report = runtime.close().expect("durability Runtime closes");
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_roots, 0);
    assert_eq!(report.remaining_interned_strings, 0);
    assert_eq!(report.remaining_estimated_bytes, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.remaining_coroutine_states, 0);
    assert_eq!(report.remaining_temporary_state_roots, 0);
    assert_eq!(report.remaining_collector_queue_entries, 0);
}
