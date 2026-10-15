//! 真实 Heap 的环境路由实验；只比较两份存储与唯一表，不模拟整个 VM。
use lua_core::{ByteString, Heap, HeapError, TableRef, Value};
use std::collections::HashMap;

pub type Install = fn(&mut Heap) -> Result<Environment, HeapError>;

pub fn key(name: &str) -> Value {
    Value::String(ByteString::new(name.as_bytes()))
}

enum Storage {
    Snapshot {
        globals: HashMap<ByteString, Value>,
        table: TableRef,
    },
    Shared(TableRef),
}

pub struct Environment {
    storage: Storage,
}

impl Environment {
    // ANCHOR: globals_snapshot_storage
    pub fn snapshot(heap: &mut Heap) -> Result<Self, HeapError> {
        let table = heap.allocate_table()?;
        let globals = HashMap::from([
            (ByteString::new(b"seed".as_slice()), Value::Number(1.0)),
            (ByteString::new(b"_G".as_slice()), Value::Table(table)),
        ]);
        for (name, value) in &globals {
            heap.table_set(table, Value::String(name.clone()), value.clone())?;
        }
        Ok(Self {
            storage: Storage::Snapshot { globals, table },
        })
    }
    // ANCHOR_END: globals_snapshot_storage

    /// 只包装既有 handle；初始化字段与后续 root 枚举由调用者各自负责。
    pub fn from_shared_table(table: TableRef) -> Self {
        Self {
            storage: Storage::Shared(table),
        }
    }

    // ANCHOR: globals_environment_roots
    fn roots(&self) -> Vec<Value> {
        match &self.storage {
            Storage::Snapshot { globals, .. } => globals.values().cloned().collect(),
            Storage::Shared(table) => vec![Value::Table(*table)],
        }
    }
    // ANCHOR_END: globals_environment_roots

    pub fn owner_table(&self) -> TableRef {
        match &self.storage {
            Storage::Snapshot { table, .. } | Storage::Shared(table) => *table,
        }
    }
}

pub struct GlobalVm {
    pub heap: Heap,
    pub environment: Environment,
}

impl GlobalVm {
    pub fn new(install: Install) -> Self {
        let mut heap = Heap::new();
        let environment = install(&mut heap).unwrap();
        Self { heap, environment }
    }

    pub fn global(&self, name: &str) -> Result<Value, HeapError> {
        match &self.environment.storage {
            Storage::Snapshot { globals, .. } => Ok(globals
                .get(&ByteString::new(name.as_bytes()))
                .cloned()
                .unwrap_or(Value::Nil)),
            Storage::Shared(table) => self.heap.table_get(*table, &key(name)),
        }
    }

    pub fn set_global(&mut self, name: &str, value: Value) -> Result<(), HeapError> {
        match &mut self.environment.storage {
            Storage::Snapshot { globals, .. } => {
                let name = ByteString::new(name.as_bytes());
                if value == Value::Nil {
                    globals.remove(&name);
                } else {
                    globals.insert(name, value);
                }
                Ok(())
            }
            Storage::Shared(table) => self.heap.table_set(*table, key(name), value),
        }
    }

    fn g_table(&self) -> TableRef {
        let Value::Table(table) = self.global("_G").unwrap() else {
            panic!("本实验只在 _G 为表时通过字段读写；不是 Lua 类型错误模型");
        };
        table
    }

    pub fn through_g(&self, name: &str) -> Result<Value, HeapError> {
        self.heap.table_get(self.g_table(), &key(name))
    }

    pub fn set_through_g(&mut self, name: &str, value: Value) -> Result<(), HeapError> {
        self.heap.table_set(self.g_table(), key(name), value)
    }

    pub fn self_cycle(&mut self) -> TableRef {
        let table = self.heap.allocate_table().unwrap();
        self.heap
            .table_set(table, key("answer"), Value::Number(42.0))
            .unwrap();
        self.heap
            .table_set(table, key("self"), Value::Table(table))
            .unwrap();
        table
    }

    pub fn collect(&mut self) {
        self.heap.collect(&self.environment.roots(), &[]).unwrap();
    }
}
