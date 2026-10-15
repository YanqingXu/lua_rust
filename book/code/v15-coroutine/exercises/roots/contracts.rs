use crate::model::{Direct, Mirror, VmModel, SUSPENDED_GRAPH, THREAD};
use crate::solution;

pub fn suspended_mirror_keeps_every_execution_value(mirror: Mirror) {
    let mut vm = VmModel::suspended();
    vm.collect(mirror, solution::extra_direct_roots, true);
    assert_eq!(vm.resume_check(), Ok(()), "{}", vm.state());
    assert_eq!(vm.mirror, [4, 8, 7, 9]);
    assert!(vm.is_live(10) && vm.is_live(11));
    // entry 与 suspended frames 是两个稳定状态，不能忘了尚未首次 resume 的 entry。
    let mut fresh = VmModel::not_started();
    fresh.collect(mirror, solution::extra_direct_roots, true);
    assert!(fresh.is_live(3) && fresh.is_live(14));
    assert_eq!(fresh.mirror, [3]);
}

pub fn direct_roots_do_not_make_host_states_permanent(direct: Direct) {
    let mut vm = VmModel::suspended();
    vm.collect(solution::mirror_roots, direct, true);
    assert_eq!(vm.resume_check(), Ok(()));
    vm.collect(solution::mirror_roots, direct, false);
    assert!(
        !vm.is_live(THREAD),
        "索引/open-upvalue 不能自举整个 thread 环：{}",
        vm.state()
    );
    assert!(
        SUSPENDED_GRAPH.iter().all(|id| !vm.is_live(*id)),
        "只清 state 不等于对象图已回收"
    );
    assert_eq!(vm.states.len(), 0);
    assert_eq!(vm.context.open_upvalues, [(100, 10)]);
    assert!(
        vm.is_live(1) && vm.is_live(10) && vm.is_live(11),
        "活动帧 upvalue 和 prefix 必须保留"
    );
}
