use crate::model::{self, BoundaryError, BoundaryHook, BoundaryVm, Callback};

pub fn restores_depth_before_returning(hook: BoundaryHook) {
    let callbacks: [(Callback, Result<i64, BoundaryError>); 3] = [
        (model::success, Ok(42)),
        (
            model::ordinary_error,
            Err(BoundaryError::Ordinary("callback failed")),
        ),
        (model::yielding_callback, Err(BoundaryError::YieldDenied)),
    ];
    for prefix in [0, 3] {
        for (callback, expected) in &callbacks {
            let mut vm = BoundaryVm::new(hook, prefix);
            let result = vm.invoke(*callback);
            assert_eq!(result, *expected, "{}", vm.summary(&result));
            assert_eq!(vm.depth, prefix, "Result must not bypass restoration");
            assert!(
                vm.pending_values.is_none() && vm.pending_frames.is_empty(),
                "denied yield cannot publish a continuation"
            );
            if prefix == 0 {
                assert_eq!(vm.try_yield(7), Err(BoundaryError::YieldSignal));
                assert_eq!(vm.pending_values, Some(vec![7]));
                assert_eq!(vm.pending_frames, ["yield-frame"]);
            } else {
                assert_eq!(vm.try_yield(7), Err(BoundaryError::YieldDenied));
                assert!(vm.pending_values.is_none() && vm.pending_frames.is_empty());
            }
        }
    }
    let mut nested = BoundaryVm::new(hook, 0);
    assert_eq!(
        nested.invoke(model::nested_after_caught_error),
        Err(BoundaryError::YieldDenied)
    );
    assert!(nested
        .trace
        .iter()
        .any(|line| line == "nested: before inner depth=1"));
    assert!(nested
        .trace
        .iter()
        .any(|line| line == "callback error depth=2"));
    assert!(nested
        .trace
        .iter()
        .any(|line| line == "nested: after inner depth=1"));
    assert_eq!(nested.depth, 0);
    assert!(nested.pending_values.is_none() && nested.pending_frames.is_empty());
    assert_eq!(nested.try_yield(17), Err(BoundaryError::YieldSignal));
    assert_eq!(
        nested.pending_values,
        Some(vec![17]),
        "old rejected payload 99 must not leak"
    );
    assert_eq!(nested.pending_frames, ["yield-frame"]);
}
