//! 生命周期观察器：实际 Heap、File、HashMap；不是生产 Vm 的替身。
use crate::support::TempDirectory;
use lua_core::{Heap, HeapError, UserdataRef, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::rc::Rc;

pub type Shutdown = fn(&mut Owner);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub stage: &'static str,
    pub entries: usize,
    pub open_files: usize,
    pub drops: [usize; 3],
    pub heap_objects: usize,
}

#[derive(Debug)]
pub struct Report {
    pub snapshots: Vec<Snapshot>,
    pub reachable_before: [bool; 3],
    pub access_rejected_after_shutdown: [bool; 3],
    pub released_objects: usize,
    pub after_owner_drops: [usize; 3],
}

#[derive(Default)]
struct Observer {
    snapshots: Vec<Snapshot>,
    drops: [usize; 3],
    access_rejected_after_shutdown: [bool; 3],
    released_objects: usize,
}

pub struct TrackedFile {
    file: Option<File>,
    label: usize,
    observer: Rc<RefCell<Observer>>,
}

// ANCHOR: file_drain_file_drop
impl Drop for TrackedFile {
    fn drop(&mut self) {
        // 先真正 drop File，再计数；观察器不克隆 File/OS handle。
        drop(self.file.take());
        self.observer.borrow_mut().drops[self.label] += 1;
    }
}
// ANCHOR_END: file_drain_file_drop

// ANCHOR: file_drain_owner
pub struct Owner {
    heap: Heap,
    pub files: HashMap<UserdataRef, Option<TrackedFile>>,
    handles: [UserdataRef; 3],
    observer: Rc<RefCell<Observer>>,
    shutdown: Shutdown,
}
// ANCHOR_END: file_drain_owner

impl Owner {
    fn observe(&self, stage: &'static str) {
        let mut observer = self.observer.borrow_mut();
        let snapshot = Snapshot {
            stage,
            entries: self.files.len(),
            open_files: self.files.values().filter(|file| file.is_some()).count(),
            drops: observer.drops,
            heap_objects: self.heap.live_count(),
        };
        observer.snapshots.push(snapshot);
    }

    // ANCHOR: file_drain_shutdown_observer
    pub fn shutdown_heap(&mut self) {
        self.observe("heap.shutdown.begin");
        let released = self.heap.shutdown();
        let rejected = self
            .handles
            .map(|handle| self.heap.userdata_metatable(handle) == Err(HeapError::Shutdown));
        {
            let mut observer = self.observer.borrow_mut();
            observer.released_objects = released;
            observer.access_rejected_after_shutdown = rejected;
        }
        self.observe("heap.shutdown.end");
    }
    // ANCHOR_END: file_drain_shutdown_observer
}

// ANCHOR: file_drain_drop_body
impl Drop for Owner {
    fn drop(&mut self) {
        self.observe("drop.body.enter");
        (self.shutdown)(self);
        self.observe("drop.body.exit");
        // 方法体返回后，Rust 才继续自动析构 Owner 各字段。
    }
}
// ANCHOR_END: file_drain_drop_body

pub fn run(shutdown: Shutdown) -> Result<Report, String> {
    let directory = TempDirectory::new()?;
    let result = (|| {
        let observer = Rc::new(RefCell::new(Observer::default()));
        let mut heap = Heap::new();
        let root = heap.allocate_table().map_err(|error| error.to_string())?;
        let handles = [
            heap.allocate_userdata()
                .map_err(|error| error.to_string())?,
            heap.allocate_userdata()
                .map_err(|error| error.to_string())?,
            heap.allocate_userdata()
                .map_err(|error| error.to_string())?,
        ];
        let mut files = HashMap::new();
        for (label, handle) in handles.iter().copied().enumerate() {
            let file = File::create(directory.path(&format!("owner-{label}.bin")))
                .map_err(|error| error.to_string())?;
            files.insert(
                handle,
                Some(TrackedFile {
                    file: Some(file),
                    label,
                    observer: observer.clone(),
                }),
            );
            heap.table_set(
                root,
                Value::Number(label as f64 + 1.0),
                Value::Userdata(handle),
            )
            .map_err(|error| error.to_string())?;
        }
        heap.collect(&[Value::Table(root)], &[])
            .map_err(|error| error.to_string())?;
        let reachable_before = std::array::from_fn(|label| {
            heap.table_get(root, &Value::Number(label as f64 + 1.0))
                == Ok(Value::Userdata(handles[label]))
                && heap.userdata_metatable(handles[label]) == Ok(None)
        });
        let mut owner = Owner {
            heap,
            files,
            handles,
            observer: observer.clone(),
            shutdown,
        };
        // 显式 close：take 留下真实 None 槽；此时该 File 已释放一次。
        drop(owner.files.get_mut(&handles[0]).unwrap().take());
        owner.observe("fixture.explicit_close");
        drop(owner); // 自定义 Drop body 和自动字段 Drop 都真实执行。
        let observer = observer.borrow();
        Ok(Report {
            snapshots: observer.snapshots.clone(),
            reachable_before,
            access_rejected_after_shutdown: observer.access_rejected_after_shutdown,
            released_objects: observer.released_objects,
            after_owner_drops: observer.drops,
        })
    })();
    directory.finish(result)
}
