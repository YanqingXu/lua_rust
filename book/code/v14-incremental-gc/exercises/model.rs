//! 只含强引用的教学模型：用于运行设计错误，不是 Heap/VM 的替代实现。
//! 分配只允许在 Pause；没有 weak、finalizer、upvalue 和宿主错误恢复。
use lua_core::{GcPhase, GcStats, GcStep};
use std::collections::BTreeMap;

pub type Id = usize;

#[derive(Clone, Debug)]
struct Node {
    name: &'static str,
    marked: bool,
    edges: Vec<Id>,
}

// ANCHOR: lesson_state
#[derive(Clone, Debug)]
pub struct StrongHeap {
    objects: BTreeMap<Id, Node>,
    next_id: Id,
    phase: GcPhase,
    gray: Vec<Id>,
    sweep_ids: Vec<Id>,
    sweep_cursor: usize,
    cycle_before: usize,
    cycle_collected: usize,
}
// ANCHOR_END: lesson_state

impl Default for StrongHeap {
    fn default() -> Self {
        Self {
            objects: BTreeMap::new(),
            next_id: 1,
            phase: GcPhase::Pause,
            gray: Vec::new(),
            sweep_ids: Vec::new(),
            sweep_cursor: 0,
            cycle_before: 0,
            cycle_collected: 0,
        }
    }
}

impl StrongHeap {
    pub fn allocate(&mut self, name: &'static str) -> Id {
        assert_eq!(self.phase, GcPhase::Pause, "模型只支持周期前分配");
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(
            id,
            Node {
                name,
                marked: false,
                edges: Vec::new(),
            },
        );
        id
    }

    pub fn is_live(&self, id: Id) -> bool {
        self.objects.contains_key(&id)
    }

    pub fn is_marked(&self, id: Id) -> bool {
        self.objects.get(&id).is_some_and(|node| node.marked)
    }

    // ANCHOR: lesson_barrier
    pub fn set_edge(&mut self, parent: Id, child: Id) {
        assert!(self.is_live(parent) && self.is_live(child));
        if self.is_marked(parent) && matches!(self.phase, GcPhase::Mark | GcPhase::Atomic) {
            self.enqueue(child);
        }
        self.set_edge_without_barrier(parent, child);
    }
    // ANCHOR_END: lesson_barrier

    /// 故障注入入口：在周期中绕过 barrier，不用于真实 Heap。
    pub fn set_edge_without_barrier(&mut self, parent: Id, child: Id) {
        assert!(self.is_live(child));
        self.objects.get_mut(&parent).unwrap().edges.push(child);
    }

    // ANCHOR: lesson_collect
    pub fn collect(&mut self, roots: &[Id]) -> GcStep {
        let mut total_work = 0;
        loop {
            let mut result = self.step(roots, usize::MAX);
            total_work += result.work;
            if result.completed.is_some() {
                result.work = total_work;
                return result;
            }
        }
    }
    // ANCHOR_END: lesson_collect

    // ANCHOR: lesson_step_entry
    pub fn step(&mut self, roots: &[Id], budget: usize) -> GcStep {
        if self.phase == GcPhase::Pause {
            self.restart_cycle();
        }
        if matches!(self.phase, GcPhase::Mark | GcPhase::Atomic) {
            for &root in roots {
                self.enqueue(root);
            }
        }
        self.advance(budget.max(1))
    }

    /// 故障注入：等价于每次调用都重建局部进度，budget=1 永远只扫描 A。
    pub fn step_restarting(&mut self, roots: &[Id], budget: usize) -> GcStep {
        self.restart_cycle();
        self.step(roots, budget)
    }
    // ANCHOR_END: lesson_step_entry

    fn restart_cycle(&mut self) {
        for node in self.objects.values_mut() {
            node.marked = false;
        }
        self.gray.clear();
        self.sweep_ids.clear();
        self.sweep_cursor = 0;
        self.cycle_before = self.objects.len();
        self.cycle_collected = 0;
        self.phase = GcPhase::Mark;
    }

    fn enqueue(&mut self, id: Id) {
        let node = self.objects.get(&id).expect("root/edge 指向已回收对象");
        if !node.marked {
            self.gray.push(id);
        }
    }

    fn mark_one(&mut self) {
        let id = self.gray.pop().unwrap();
        let node = self.objects.get_mut(&id).unwrap();
        if node.marked {
            return;
        }
        node.marked = true;
        let edges = node.edges.clone();
        for child in edges {
            self.enqueue(child);
        }
    }

    // ANCHOR: lesson_advance
    fn advance(&mut self, budget: usize) -> GcStep {
        let mut work = 0;
        let mut completed = None;
        match self.phase {
            GcPhase::Pause => unreachable!(),
            GcPhase::Mark => {
                while work < budget && !self.gray.is_empty() {
                    self.mark_one();
                    work += 1;
                }
                if self.gray.is_empty() {
                    self.phase = GcPhase::Atomic;
                }
            }
            GcPhase::Atomic => {
                while !self.gray.is_empty() {
                    self.mark_one();
                }
                self.sweep_ids = self.objects.keys().copied().collect();
                self.phase = GcPhase::Sweep;
                work = 1;
            }
            GcPhase::Sweep => {
                while work < budget && self.sweep_cursor < self.sweep_ids.len() {
                    let id = self.sweep_ids[self.sweep_cursor];
                    self.sweep_cursor += 1;
                    if !self.is_marked(id) {
                        self.objects.remove(&id);
                        self.cycle_collected += 1;
                    }
                    work += 1;
                }
                if self.sweep_cursor == self.sweep_ids.len() {
                    self.phase = GcPhase::Pause;
                    for node in self.objects.values_mut() {
                        node.marked = false;
                    }
                    completed = Some(GcStats {
                        before: self.cycle_before,
                        collected: self.cycle_collected,
                        remaining: self.objects.len(),
                    });
                }
            }
        }
        GcStep {
            phase: self.phase,
            work,
            completed,
        }
    }
    // ANCHOR_END: lesson_advance

    /// 白=未标记且不在队列；灰=待扫描；黑=已扫描。只用于可视化模型状态。
    pub fn state(&self) -> String {
        let colors = self
            .objects
            .iter()
            .map(|(id, node)| {
                let color = if node.marked {
                    "black"
                } else if self.gray.contains(id) {
                    "gray"
                } else {
                    "white"
                };
                format!("{}:{color}", node.name)
            })
            .collect::<Vec<_>>()
            .join(",");
        let gray = self
            .gray
            .iter()
            .map(|id| self.objects[id].name)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "phase={:?} gray=[{gray}] sweep={}/{} objects=[{colors}]",
            self.phase,
            self.sweep_cursor,
            self.sweep_ids.len()
        )
    }
}

pub fn chain() -> (StrongHeap, Id, Id, Id, Id) {
    let mut heap = StrongHeap::default();
    let a = heap.allocate("A");
    let b = heap.allocate("B");
    let c = heap.allocate("C");
    let d = heap.allocate("D");
    heap.set_edge(a, b);
    heap.set_edge(b, c);
    heap.set_edge(d, d);
    (heap, a, b, c, d)
}

pub fn pair() -> (StrongHeap, Id, Id) {
    let mut heap = StrongHeap::default();
    let a = heap.allocate("A");
    let b = heap.allocate("B");
    (heap, a, b)
}
