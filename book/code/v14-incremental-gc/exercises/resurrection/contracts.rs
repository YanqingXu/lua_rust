use crate::model::{AtomicOrder, ResurrectionHeap};

pub fn pending_graph_precedes_weak_cleanup(order: AtomicOrder) {
    let mut heap = ResurrectionHeap::default();
    let first = heap.cycle(order, true);
    assert_eq!(first.pending_before_callback, 1);
    assert_eq!(first.live_after_sweep, [true; 4]);
    assert_eq!(
        first.weak_before_callback, [true; 4],
        "先清理会永久删除仍应保留的 weak-value 条目"
    );
    assert_eq!(first.callback_saw_weak, Some([true; 4]));
    assert!(first.prefix_live && first.rescue_root);
    assert_eq!((first.finalized, first.total_callbacks), (Some(true), 1));
    let cleanup = heap
        .trace
        .iter()
        .position(|line| line.contains("cleanup"))
        .unwrap();
    let callback = heap
        .trace
        .iter()
        .position(|line| line.contains("invoke callback"))
        .unwrap();
    assert!(cleanup < callback, "callback 必须在 Heap 清理并返回后运行");

    let second = heap.cycle(order, true);
    assert_eq!(second.pending_before_callback, 0);
    assert_eq!(second.weak_before_callback, [true; 4]);
    assert_eq!(second.live_after_sweep, [true; 4]);
    assert_eq!(second.callback_saw_weak, None);
    assert_eq!((second.finalized, second.total_callbacks), (Some(true), 1));
    assert!(second.prefix_live && second.rescue_root);

    heap.remove_rescue_root();
    let third = heap.cycle(order, true);
    assert_eq!(third.pending_before_callback, 0);
    assert_eq!(third.weak_before_callback, [false; 4]);
    assert_eq!(third.live_after_sweep, [false; 4]);
    assert_eq!((third.finalized, third.total_callbacks), (None, 1));
    assert!(third.prefix_live && !third.rescue_root);
}

pub fn no_rescue_dies_next_cycle(order: AtomicOrder) {
    let mut heap = ResurrectionHeap::default();
    let first = heap.cycle(order, false);
    assert_eq!(first.weak_before_callback, [true; 4]);
    assert_eq!(first.live_after_sweep, [true; 4]);
    assert_eq!(first.total_callbacks, 1);
    assert!(!first.rescue_root);
    let second = heap.cycle(order, false);
    assert_eq!(second.pending_before_callback, 0);
    assert_eq!(second.weak_before_callback, [false; 4]);
    assert_eq!(second.live_after_sweep, [false; 4]);
    assert_eq!(second.total_callbacks, 1);
    assert!(second.prefix_live);
}
