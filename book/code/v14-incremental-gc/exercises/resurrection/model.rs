//! 单个 finalizable userdata 的 weak-value 清理顺序模型；不模拟 ephemeron。
use std::collections::{BTreeMap, BTreeSet};

pub type Id = usize;
pub type AtomicOrder = fn(&mut ResurrectionHeap);
const PREFIX: Id = 1;
const WEAK: Id = 2;
const USERDATA: Id = 3;
const CALLBACK: Id = 4;
const CHILD: Id = 5;
const GRANDCHILD: Id = 6;
const GRAPH: [Id; 4] = [USERDATA, CALLBACK, CHILD, GRANDCHILD];

#[derive(Debug, PartialEq, Eq)]
pub struct Cycle {
    pub pending_before_callback: usize,
    pub weak_before_callback: [bool; 4],
    pub callback_saw_weak: Option<[bool; 4]>,
    pub live_after_sweep: [bool; 4],
    pub total_callbacks: usize,
    pub rescue_root: bool,
    pub finalized: Option<bool>,
    pub prefix_live: bool,
}

pub struct ResurrectionHeap {
    objects: BTreeMap<Id, Vec<Id>>,
    weak_entries: BTreeMap<Id, Id>,
    marked: BTreeSet<Id>,
    pending: Vec<Id>,
    finalized: bool,
    rescue: Option<Id>,
    callbacks: usize,
    pub trace: Vec<String>,
}

impl Default for ResurrectionHeap {
    fn default() -> Self {
        Self {
            objects: BTreeMap::from([
                (PREFIX, vec![]),
                (WEAK, vec![]),
                (USERDATA, vec![CALLBACK]),
                (CALLBACK, vec![CHILD]),
                (CHILD, vec![GRANDCHILD]),
                (GRANDCHILD, vec![]),
            ]),
            weak_entries: GRAPH.into_iter().map(|id| (id, id)).collect(),
            marked: BTreeSet::new(),
            pending: Vec::new(),
            finalized: false,
            rescue: None,
            callbacks: 0,
            trace: Vec::new(),
        }
    }
}

impl ResurrectionHeap {
    /// 在正式清理 weak 之前，传播待处理 userdata/callback 的全部强图。
    pub fn mark_pending_graph(&mut self) {
        self.trace
            .push("Heap: mark pending strong graph".to_owned());
        let mut roots = self.pending.clone();
        if !self.pending.is_empty() {
            roots.push(CALLBACK);
        }
        self.mark_from(roots);
    }

    pub fn cleanup_weak(&mut self) {
        self.trace.push("Heap: cleanup weak values".to_owned());
        self.weak_entries
            .retain(|_, value| self.marked.contains(value));
    }

    fn mark_from(&mut self, mut gray: Vec<Id>) {
        while let Some(id) = gray.pop() {
            if self.marked.insert(id) {
                gray.extend(self.objects[&id].iter().copied());
            }
        }
    }

    pub fn remove_rescue_root(&mut self) {
        self.rescue = None;
    }

    fn weak_presence(&self) -> [bool; 4] {
        GRAPH.map(|id| self.weak_entries.contains_key(&id))
    }

    pub fn cycle(&mut self, order: AtomicOrder, rescue_in_callback: bool) -> Cycle {
        assert!(self.pending.is_empty(), "上批事件必须已消费");
        self.marked.clear();
        let mut roots = vec![PREFIX, WEAK];
        roots.extend(self.rescue);
        self.mark_from(roots);
        if self.objects.contains_key(&USERDATA)
            && !self.marked.contains(&USERDATA)
            && !self.finalized
        {
            self.finalized = true;
            self.pending.push(USERDATA);
            self.trace
                .push("Heap: set finalized; enqueue pending event".to_owned());
        }
        order(self);
        self.trace
            .push("Heap: prepare Sweep; return to VM".to_owned());
        let pending_before_callback = self.pending.len();
        let weak_before_callback = self.weak_presence();
        let mut callback_saw_weak = None;
        // 只有 Heap 完成清理并返回后，VM 才能调用 callback / 发布 rescue root。
        for userdata in std::mem::take(&mut self.pending) {
            self.callbacks += 1;
            callback_saw_weak = Some(self.weak_presence());
            self.trace.push("VM: invoke callback".to_owned());
            if rescue_in_callback {
                self.rescue = Some(userdata);
                self.trace.push("VM: publish rescue root".to_owned());
            }
        }
        self.objects.retain(|id, _| self.marked.contains(id));
        Cycle {
            pending_before_callback,
            weak_before_callback,
            callback_saw_weak,
            live_after_sweep: GRAPH.map(|id| self.objects.contains_key(&id)),
            total_callbacks: self.callbacks,
            rescue_root: self.rescue.is_some(),
            finalized: self
                .objects
                .contains_key(&USERDATA)
                .then_some(self.finalized),
            prefix_live: self.objects.contains_key(&PREFIX) && self.objects.contains_key(&WEAK),
        }
    }
}
