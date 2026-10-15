//! Real typed userdata and File ownership; no parser, registry pruning or shutdown model.
use lua_core::{ByteString, Heap, UserdataRef, Value};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::rc::Rc;

#[derive(Debug, Eq, PartialEq)]
pub enum FileError {
    NotFile,
    AlreadyClosed,
    UseClosed,
    Io(String),
}

impl FileError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFile => "Type",
            _ => "Runtime",
        }
    }
}

pub struct OwnedFile {
    pub file: File,
    dropped: Rc<Cell<usize>>,
}

impl Drop for OwnedFile {
    fn drop(&mut self) {
        self.dropped.set(self.dropped.get() + 1);
        // The File field is then dropped by Rust, closing this owned handle.
    }
}

pub type Close = fn(&mut FileVm, UserdataRef) -> Result<(), FileError>;

pub struct FileVm {
    pub heap: Heap,
    pub files: HashMap<UserdataRef, Option<OwnedFile>>,
    /// Only the fixed flag-only fault writes here; healthy state needs no flag.
    pub claimed_closed: HashSet<UserdataRef>,
}

impl FileVm {
    pub fn new() -> Self {
        Self {
            heap: Heap::new(),
            files: HashMap::new(),
            claimed_closed: HashSet::new(),
        }
    }

    // ANCHOR: file_close_registration
    pub fn register(&mut self, file: File) -> (UserdataRef, Rc<Cell<usize>>) {
        let userdata = self.heap.allocate_userdata().unwrap();
        let metatable = self.heap.allocate_table().unwrap();
        let metadata = self.heap.allocate_table().unwrap();
        self.heap
            .table_set(metadata, key("answer"), Value::Number(42.0))
            .unwrap();
        self.heap
            .table_set(metatable, key("metadata"), Value::Table(metadata))
            .unwrap();
        self.heap
            .userdata_set_metatable(userdata, Some(metatable))
            .unwrap();
        let dropped = Rc::new(Cell::new(0));
        self.files.insert(
            userdata,
            Some(OwnedFile {
                file,
                dropped: dropped.clone(),
            }),
        );
        (userdata, dropped)
    }
    // ANCHOR_END: file_close_registration

    pub fn io_type(&self, handle: UserdataRef) -> Option<&'static str> {
        self.files.get(&handle).map(|entry| {
            if entry.is_some() {
                "file"
            } else {
                "closed file"
            }
        })
    }

    pub fn metadata_answer(&self, handle: UserdataRef) -> Value {
        let metatable = self.heap.userdata_metatable(handle).unwrap().unwrap();
        let Value::Table(metadata) = self.heap.table_get(metatable, &key("metadata")).unwrap()
        else {
            panic!("metadata table must remain reachable through userdata");
        };
        self.heap.table_get(metadata, &key("answer")).unwrap()
    }

    fn open(&mut self, handle: UserdataRef) -> Result<&mut File, FileError> {
        self.files
            .get_mut(&handle)
            .ok_or(FileError::NotFile)?
            .as_mut()
            .map(|owner| &mut owner.file)
            .ok_or(FileError::UseClosed)
    }

    pub fn read(&mut self, handle: UserdataRef) -> Result<Vec<u8>, FileError> {
        let mut bytes = Vec::new();
        self.open(handle)?
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        Ok(bytes)
    }

    pub fn write(&mut self, handle: UserdataRef, bytes: &[u8]) -> Result<(), FileError> {
        self.open(handle)?.write_all(bytes).map_err(io_error)
    }

    pub fn seek(&mut self, handle: UserdataRef) -> Result<u64, FileError> {
        self.open(handle)?
            .seek(SeekFrom::Start(0))
            .map_err(io_error)
    }

    pub fn flush(&mut self, handle: UserdataRef) -> Result<(), FileError> {
        self.open(handle)?.flush().map_err(io_error)
    }
}

fn io_error(error: std::io::Error) -> FileError {
    FileError::Io(error.to_string())
}

fn key(name: &str) -> Value {
    Value::String(ByteString::new(name.as_bytes()))
}
