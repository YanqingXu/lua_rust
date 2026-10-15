//! 稳定 suspended 边界的 tracing 模型，不执行字节码、不模拟 yield 的展开间隙。
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

pub type Id = usize;
pub type FrameId = usize;
pub type Mirror = fn(&State) -> Vec<Id>;
pub type Direct = fn(&RootContext) -> Vec<Id>;
pub const THREAD: Id = 2;
pub const SUSPENDED_GRAPH: [Id; 9] = [2, 4, 5, 6, 7, 8, 9, 12, 13];

pub struct Frame {
    pub function: Id,
    pub registers: Vec<Rc<RefCell<Id>>>,
    pub varargs: Vec<Id>,
}

pub struct State {
    pub thread: Id,
    pub owner: FrameId,
    pub entry: Option<Id>,
    pub frames: Vec<Frame>,
}

pub struct RootContext {
    pub active_frames: BTreeSet<FrameId>,
    pub open_upvalues: Vec<(FrameId, Id)>,
    pub scheduler_handles: Vec<Id>,
}

pub struct VmModel {
    pub states: BTreeMap<Id, State>,
    pub context: RootContext,
    objects: BTreeMap<Id, Vec<Id>>,
    pub mirror: Vec<Id>,
}

impl VmModel {
    pub fn suspended() -> Self {
        let state = State {
            thread: THREAD,
            owner: 42,
            entry: None,
            frames: vec![Frame {
                function: 4,
                registers: vec![Rc::new(RefCell::new(7)), Rc::new(RefCell::new(9))],
                varargs: vec![8],
            }],
        };
        Self {
            states: BTreeMap::from([(THREAD, state)]),
            context: RootContext {
                active_frames: BTreeSet::from([100]),
                open_upvalues: vec![(42, 5), (100, 10)],
                scheduler_handles: vec![THREAD],
            },
            objects: BTreeMap::from([
                (1, vec![]),   // 长期 prefix root
                (2, vec![]),   // Heap thread 的 Value 镜像；不是执行 slots
                (4, vec![12]), // 暂停函数自己的 closed capture
                (12, vec![13]),
                (13, vec![]),
                (7, vec![]),       // 仅寄存器可达的 table
                (8, vec![]),       // 仅暂停 varargs 可达的 table
                (9, vec![5]),      // 暂停寄存器中的 getter closure
                (5, vec![6]),      // getter 的 open upvalue → slot 当前内容
                (6, vec![THREAD]), // cycle.thread；全 upvalue 直根会保活整环
                (10, vec![11]),    // 运行帧的 open upvalue 必须仍作为直接 root
                (11, vec![]),
            ]),
            mirror: Vec::new(),
        }
    }

    pub fn not_started() -> Self {
        let mut vm = Self::suspended();
        let state = vm.states.get_mut(&THREAD).unwrap();
        state.entry = Some(3);
        state.frames.clear();
        vm.objects.insert(3, vec![14]);
        vm.objects.insert(14, vec![]);
        vm.context.open_upvalues.retain(|(owner, _)| *owner == 100);
        vm
    }

    pub fn collect(&mut self, mirror: Mirror, direct: Direct, keep_handle: bool) {
        // 同步发生在下次可收集前；此模型不规定 detach/sync 的源码先后次序。
        for state in self.states.values() {
            let values = mirror(state);
            self.objects
                .get_mut(&state.thread)
                .unwrap()
                .clone_from(&values);
            self.mirror = values;
        }
        let mut gray = vec![1];
        if keep_handle {
            gray.push(THREAD);
        }
        gray.extend(direct(&self.context));
        let mut marked = BTreeSet::new();
        while let Some(id) = gray.pop() {
            if marked.insert(id) {
                gray.extend(self.objects[&id].iter().copied());
            }
        }
        self.objects.retain(|id, _| marked.contains(id));
        // 对应完整 Sweep 后 prune：清 VM 执行 state 及其 open-upvalue 索引。
        let dead_owners = self
            .states
            .values()
            .filter(|state| !self.objects.contains_key(&state.thread))
            .map(|state| state.owner)
            .collect::<Vec<_>>();
        self.states.retain(|id, _| self.objects.contains_key(id));
        self.context
            .open_upvalues
            .retain(|(owner, _)| !dead_owners.contains(owner));
        self.context
            .scheduler_handles
            .retain(|id| self.objects.contains_key(id));
    }

    pub fn is_live(&self, id: Id) -> bool {
        self.objects.contains_key(&id)
    }

    pub fn resume_check(&self) -> Result<(), &'static str> {
        if SUSPENDED_GRAPH.iter().all(|id| self.is_live(*id)) {
            Ok(())
        } else {
            Err("stale saved function/register/vararg/capture")
        }
    }

    pub fn state(&self) -> String {
        format!(
            "mirror={:?} live={:?} states={} open_upvalues={:?}",
            self.mirror,
            self.objects.keys().collect::<Vec<_>>(),
            self.states.len(),
            self.context.open_upvalues
        )
    }
}
