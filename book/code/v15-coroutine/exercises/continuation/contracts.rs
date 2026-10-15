use crate::model::{
    ChildYield, ContinuationVm, Design, Outcome, ResumePc, SaveFrame, Status, TurnVm, Value,
};
use crate::solution;

pub fn healthy() -> Design {
    Design {
        save: solution::save_frame,
        pc: solution::resume_pc,
    }
}

fn yielded(number: i64) -> Result<Outcome, crate::model::ResumeError> {
    Ok(Outcome::Yield(vec![Value::Number(number), Value::Nil]))
}

fn returned(number: i64) -> Result<Outcome, crate::model::ResumeError> {
    Ok(Outcome::Return(vec![Value::Number(number), Value::Nil]))
}

pub fn per_coroutine_storage(save: SaveFrame) {
    let mut vm = ContinuationVm::new(Design { save, ..healthy() }, &[(1, 10), (2, 100)]);
    assert_eq!(vm.resume(1, 0), yielded(10));
    assert_eq!(vm.resume(2, 0), yielded(100));
    assert_eq!(
        vm.coroutines[&1].frame.as_ref().map(|frame| frame.local),
        Some(10)
    );
    assert_eq!(
        vm.coroutines[&2].frame.as_ref().map(|frame| frame.local),
        Some(100)
    );
    assert_eq!(vm.resume(1, 1), yielded(11));
    assert_eq!(vm.resume(2, 2), yielded(102));
    assert_eq!(vm.resume(1, 3), returned(14));
    assert_eq!(vm.resume(2, 4), returned(106));
    assert_eq!(vm.starts.values().copied().collect::<Vec<_>>(), [1, 1]);
    assert!(vm.scratch.is_none());
    assert_eq!(vm.coroutines[&1].status, Status::Dead);
    assert_eq!(vm.coroutines[&2].status, Status::Dead);
}

pub fn resumes_after_yield(pc: ResumePc) {
    let mut vm = ContinuationVm::new(Design { pc, ..healthy() }, &[(1, 10)]);
    assert_eq!(vm.resume(1, 0), yielded(10));
    assert_eq!(vm.resume(1, 5), yielded(15));
    assert_eq!(vm.resume(1, 7), returned(22));
    assert_eq!(vm.starts[&1], 1, "entry side effect must run once");
    assert_eq!(vm.coroutines[&1].status, Status::Dead);
}

pub fn restores_parent_turn(run: fn(&mut TurnVm) -> ChildYield) {
    let mut vm = TurnVm::new();
    let child = run(&mut vm);
    assert_eq!(vm.observed_parent, Some(Status::Normal));
    assert_eq!(vm.observed_current, Some(1));
    assert!(vm.child_saw_empty_channels);
    assert_eq!(
        child.values,
        [Value::Number(7), Value::Nil, Value::Number(9)]
    );
    assert_eq!(child.frames, ["child-frame"]);
    assert_eq!(vm.current, 0);
    assert_eq!(vm.statuses, [Status::Running, Status::Suspended]);
    assert_eq!(vm.yield_values, Some(vec![Value::Number(900)]));
    assert_eq!(vm.yield_frames, ["outer-frame"]);
}
