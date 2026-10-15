use crate::model::{CallbackVm, Design, Event, EventError, Handle};
use crate::solution;

pub fn healthy() -> Design {
    Design {
        take: solution::take_batch,
        pin: solution::pin_batch,
        drain: solution::drain_batch,
    }
}

pub fn transfers_batch(take: fn(&mut Vec<Event>) -> Vec<Event>) {
    let mut vm = CallbackVm::reentrant_pair(Design { take, ..healthy() });
    let result = vm.drain_pending();
    assert_eq!(result, Ok(()), "{}", vm.summary(&result));
    assert_eq!(vm.calls, [1, 2], "重入不能重复看到外层正在处理的事件");
    assert!(vm.pending.is_empty(), "事件应从 Heap 队列移到 owned batch");
    assert_eq!(vm.roots(), [900]);
}

pub fn protects_sibling_events(pin: fn(&mut Vec<Handle>, &[Event])) {
    let mut vm = CallbackVm::reentrant_pair(Design { pin, ..healthy() });
    assert_eq!(vm.drain_pending(), Ok(()), "兄弟事件不能变成 stale handle");
    assert_eq!(vm.calls, [1, 2]);
    assert!(vm.is_live(2) && vm.is_live(102));
    assert_eq!(vm.roots(), [900], "批次结束后必须保留外层原有 root");
    vm.collect();
    assert!(!vm.is_live(2) && !vm.is_live(102), "临时 roots 不能泄漏");
}

pub fn drains_after_error(drain: fn(&mut CallbackVm, Vec<Event>) -> Result<(), EventError>) {
    let mut vm = CallbackVm::failing_trio(Design { drain, ..healthy() });
    assert_eq!(vm.drain_pending(), Err(EventError::Callback(1)));
    assert_eq!(vm.calls, [1, 2, 3], "一个失败不能跳过后续事件");
    assert!(vm.pending.is_empty());
    assert_eq!(vm.roots(), [900], "错误返回前也必须 truncate 到进入时长度");
    vm.collect();
    assert!(!vm.is_live(1) && !vm.is_live(2) && !vm.is_live(3));
}
