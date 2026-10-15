use crate::temp::TempFiles;
use lua_core::{GcStep, Heap, UserdataRef, Value};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Exclusive {
    Blocked,
    Available,
    Unsupported,
}

// This checks the OS handle, rather than inferring close from file deletion.
pub fn exclusive(path: &Path) -> Result<Exclusive, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        match OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
        {
            Ok(file) => {
                drop(file);
                Ok(Exclusive::Available)
            }
            Err(error) if error.raw_os_error() == Some(32) => Ok(Exclusive::Blocked),
            Err(error) => Err(format!("exclusive reopen {}: {error}", path.display())),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(Exclusive::Unsupported)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct View {
    pub live: [bool; 4],
    pub accessible: [bool; 4],
    pub entries: [bool; 4],
    pub open: [bool; 4],
    pub exclusive: [Exclusive; 4],
}

// ANCHOR: file_prune_ownership
pub struct FileModel {
    pub heap: Heap,
    pub files: HashMap<UserdataRef, Option<File>>,
    pub handles: [UserdataRef; 4],
    pub roots: Vec<Value>,
    pub paths: [PathBuf; 4],
}

impl FileModel {
    // Keys and handles are Rust-owned copies, not tracing roots.
    pub fn step(&mut self, root_all_keys: bool) -> Result<GcStep, String> {
        let mut roots = self.roots.clone();
        if root_all_keys {
            roots.extend(self.files.keys().copied().map(Value::Userdata));
        }
        self.heap.step(&roots, &[], 1).map_err(|e| e.to_string())
    }
    // ANCHOR_END: file_prune_ownership

    pub fn finish_cycle(&mut self, root_all_keys: bool) -> Result<GcStep, String> {
        for _ in 0..64 {
            let step = self.step(root_all_keys)?;
            if step.completed.is_some() {
                return Ok(step);
            }
        }
        Err("fixture cycle exceeded 64 steps".to_owned())
    }

    pub fn partial_sweep(&mut self) -> Result<GcStep, String> {
        for _ in 0..64 {
            let step = self.step(false)?;
            if self.heap.is_live(self.handles[2]) == Ok(false) && step.completed.is_none() {
                return Ok(step);
            }
            if step.completed.is_some() {
                return Err("expected a partial Sweep before cycle completion".to_owned());
            }
        }
        Err("partial Sweep exceeded 64 steps".to_owned())
    }

    pub fn view(&self) -> Result<View, String> {
        let mut exclusive_states = [Exclusive::Unsupported; 4];
        for (index, path) in self.paths.iter().enumerate() {
            exclusive_states[index] = exclusive(path)?;
        }
        Ok(View {
            live: self.handles.map(|h| self.heap.is_live(h) == Ok(true)),
            accessible: self
                .handles
                .map(|h| self.heap.userdata_is_finalized(h) == Ok(false)),
            entries: self.handles.map(|h| self.files.contains_key(&h)),
            open: self
                .handles
                .map(|h| self.files.get(&h).is_some_and(Option::is_some)),
            exclusive: exclusive_states,
        })
    }
}

pub type Prune = fn(&mut FileModel, &GcStep);

pub fn with_fixture<T>(
    body: impl FnOnce(&mut FileModel) -> Result<T, String>,
) -> Result<T, String> {
    let directory = TempFiles::new()?;
    let result = (|| {
        let mut heap = Heap::new();
        let mut allocate = || heap.allocate_userdata().map_err(|e| e.to_string());
        let handles = [allocate()?, allocate()?, allocate()?, allocate()?];
        let paths = std::array::from_fn(|index| directory.file(index));
        let mut files = HashMap::new();
        for (index, handle) in handles.iter().enumerate() {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&paths[index])
                .map_err(|e| e.to_string())?;
            let retained = if index % 2 == 0 {
                Some(file)
            } else {
                drop(file);
                None
            };
            files.insert(*handle, retained);
        }
        let mut model = FileModel {
            heap,
            files,
            handles,
            paths,
            roots: handles[..2].iter().copied().map(Value::Userdata).collect(),
        };
        body(&mut model)
        // model drops (including every File) before checked directory cleanup.
    })();
    directory.finish(result)
}
