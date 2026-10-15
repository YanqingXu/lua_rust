//! 已经排好 finalizer 事件后的独立小模型，不模拟 Atomic/weak/完整 VM。
//! Event 中只有整数 handle：Rust 拥有 Vec 不等于 GC 能从它追踪到对象。
use std::collections::{BTreeMap, BTreeSet};

pub type Handle = usize;

#[derive(Clone, Debug)]
pub struct Event {
    pub number: usize,
    pub userdata: Handle,
    pub callback: Handle,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventError {
    Stale(usize),
    Callback(usize),
}

#[derive(Clone, Copy)]
pub struct Design {
    pub take: fn(&mut Vec<Event>) -> Vec<Event>,
    pub pin: fn(&mut Vec<Handle>, &[Event]),
    pub drain: fn(&mut CallbackVm, Vec<Event>) -> Result<(), EventError>,
}

pub struct CallbackVm {
    pub pending: Vec<Event>,
    pub calls: Vec<usize>,
    pub trace: Vec<String>,
    temporary_roots: Vec<Handle>,
    live: BTreeSet<Handle>,
    edges: BTreeMap<Handle, Vec<Handle>>,
    failing: BTreeSet<usize>,
    reentries_left: usize,
    design: Design,
}

impl CallbackVm {
    pub fn reentrant_pair(design: Design) -> Self {
        Self::new(design, 2, true, &[])
    }

    pub fn failing_trio(design: Design) -> Self {
        Self::new(design, 3, false, &[1, 2])
    }

    fn new(design: Design, count: usize, reentrant: bool, failing: &[usize]) -> Self {
        let pending = (1..=count)
            .map(|number| Event {
                number,
                userdata: number,
                callback: 100 + number,
            })
            .collect::<Vec<_>>();
        let mut live = BTreeSet::from([900]);
        let mut edges = BTreeMap::new();
        for event in &pending {
            live.extend([event.userdata, event.callback]);
            edges.insert(event.userdata, vec![event.callback]);
        }
        Self {
            pending,
            calls: Vec::new(),
            trace: Vec::new(),
            // 900 模拟外层调用已经放好的临时 root；内层不能 clear 全部 roots。
            temporary_roots: vec![900],
            live,
            edges,
            failing: failing.iter().copied().collect(),
            reentries_left: usize::from(reentrant),
            design,
        }
    }

    pub fn drain_pending(&mut self) -> Result<(), EventError> {
        let batch = (self.design.take)(&mut self.pending);
        self.trace.push(format!(
            "take batch={} pending={}",
            batch.len(),
            self.pending.len()
        ));
        (self.design.drain)(self, batch)
    }

    pub fn root_len(&self) -> usize {
        self.temporary_roots.len()
    }

    pub fn roots(&self) -> &[Handle] {
        &self.temporary_roots
    }

    pub fn pin_batch(&mut self, events: &[Event]) {
        (self.design.pin)(&mut self.temporary_roots, events);
        self.trace
            .push(format!("pin roots={:?}", self.temporary_roots));
    }

    pub fn truncate_roots(&mut self, base: usize) {
        self.temporary_roots.truncate(base);
    }

    pub fn is_live(&self, handle: Handle) -> bool {
        self.live.contains(&handle)
    }

    pub fn invoke(&mut self, event: Event) -> Result<(), EventError> {
        if !self.is_live(event.userdata) || !self.is_live(event.callback) {
            self.trace.push(format!("stale event={}", event.number));
            return Err(EventError::Stale(event.number));
        }
        self.calls.push(event.number);
        self.trace.push(format!("call event={}", event.number));
        // 调用中的对象由活动调用帧保活；尚未调用的兄弟事件没有这层保护。
        let active_base = self.root_len();
        self.temporary_roots
            .extend([event.callback, event.userdata]);
        let result = if event.number == 1 && self.reentries_left > 0 {
            // 故障重放总共只准重入一次，clone 坏设计不会无限递归。
            self.reentries_left -= 1;
            self.collect();
            self.drain_pending()
        } else {
            Ok(())
        };
        self.truncate_roots(active_base);
        result?;
        if self.failing.contains(&event.number) {
            return Err(EventError::Callback(event.number));
        }
        Ok(())
    }

    /// 只追踪显式 roots 和 userdata→callback 边；完全不检查 Rust 栈的 Vec。
    pub fn collect(&mut self) {
        let mut gray = self.temporary_roots.clone();
        let mut marked = BTreeSet::new();
        while let Some(handle) = gray.pop() {
            if marked.insert(handle) {
                if let Some(edges) = self.edges.get(&handle) {
                    gray.extend(edges.iter().copied());
                }
            }
        }
        let before = self.live.len();
        self.live.retain(|handle| marked.contains(handle));
        self.trace.push(format!(
            "gc roots={:?} reclaimed={}",
            self.temporary_roots,
            before - self.live.len()
        ));
    }

    pub fn summary(&self, result: &Result<(), EventError>) -> String {
        format!(
            "calls={:?} pending={} roots={:?} result={result:?}",
            self.calls,
            self.pending.len(),
            self.temporary_roots
        )
    }
}
