use crate::model::{chain, pair, Id, StrongHeap};
use lua_core::{GcPhase, GcStats, GcStep};

pub type StepFn = fn(&mut StrongHeap, &[Id], usize) -> GcStep;

pub fn bounded_first_step(step: StepFn) {
    let (mut heap, a, b, c, d) = chain();
    let first = step(&mut heap, &[a], 1);
    assert!(
        first.completed.is_none(),
        "首次必须交还控制权，不能已完成整个周期"
    );
    assert_eq!((first.phase, first.work), (GcPhase::Mark, 1));
    assert!(heap.is_live(a) && heap.is_live(b) && heap.is_live(c) && heap.is_live(d));
    assert!(heap.is_marked(a) && !heap.is_marked(b));
}

pub fn progress_survives_calls(step: StepFn) {
    let (mut full, root, _, _, _) = chain();
    let expected = full.collect(&[root]).completed.unwrap();
    let (mut heap, a, b, c, d) = chain();
    let mut completed = None;
    let mut phases = Vec::new();
    // 3 次 Mark + 1 次 Atomic + 4 次 Sweep；重启错误在 8 次上界内暴露。
    for _ in 0..8 {
        let report = step(&mut heap, &[a], 1);
        assert!(report.work <= 1);
        phases.push(report.phase);
        if report.completed.is_some() {
            completed = report.completed;
            break;
        }
    }
    assert_eq!(completed, Some(expected), "每次重建进度会永远停在 Mark");
    assert!(phases.contains(&GcPhase::Atomic) && phases.contains(&GcPhase::Sweep));
    assert!(heap.is_live(a) && heap.is_live(b) && heap.is_live(c));
    assert!(!heap.is_live(d));
    assert_eq!(heap.collect(&[]).completed.unwrap().remaining, 0);
}

pub fn late_edge_keeps_white_child(connect: fn(&mut StrongHeap, Id, Id)) {
    let (mut heap, a, b) = pair();
    let first = heap.step(&[a], 1);
    assert_eq!(first.phase, GcPhase::Atomic);
    assert!(heap.is_marked(a) && !heap.is_marked(b));
    connect(&mut heap, a, b);
    assert_eq!(
        heap.collect(&[a]).completed,
        Some(GcStats {
            before: 2,
            collected: 0,
            remaining: 2
        })
    );
    assert!(
        heap.is_live(b),
        "root→已扫描 parent→新接入 child 必须仍可达"
    );
    // barrier 只保护当前周期；删除全部 roots 后不能让两个对象永久存活。
    assert_eq!(heap.collect(&[]).completed.unwrap().remaining, 0);
}
