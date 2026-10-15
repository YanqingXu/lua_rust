//! 有界 require 循环教学：真实 Heap loaded/exports；不模拟文件搜索和错误回滚。
use lua_core::{ByteString, Heap, HeapError, TableRef, Value};
use std::collections::BTreeMap;

pub type Name = &'static str;
pub type Loader = fn(&mut CycleVm, Name) -> Result<Value, LoadError>;
pub type RunLoader = fn(&mut CycleVm, Name, Loader) -> Result<Value, LoadError>;

#[derive(Debug, PartialEq)]
pub enum LoadError {
    Heap(HeapError),
    Budget,
}
impl From<HeapError> for LoadError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

pub fn key(name: &str) -> Value {
    Value::String(ByteString::new(name.as_bytes()))
}

pub struct CycleVm {
    pub heap: Heap,
    pub loaded: TableRef,
    pub calls: BTreeMap<Name, usize>,
    pub trace: Vec<String>,
    remaining: usize,
    run_loader: RunLoader,
    loaders: BTreeMap<Name, Loader>,
}

impl CycleVm {
    pub fn new(run_loader: RunLoader, budget: usize) -> Self {
        let mut heap = Heap::new();
        let loaded = heap.allocate_table().unwrap();
        Self {
            heap,
            loaded,
            calls: BTreeMap::new(),
            trace: Vec::new(),
            remaining: budget,
            run_loader,
            loaders: BTreeMap::from([
                ("self", self_loader as Loader),
                ("A", a_loader as Loader),
                ("B", b_loader as Loader),
                ("partial", partial_loader as Loader),
                ("done_true", true_loader as Loader),
            ]),
        }
    }

    // ANCHOR: cycle_require_lookup
    pub fn require(&mut self, name: Name) -> Result<Value, LoadError> {
        let loaded = self.heap.table_get(self.loaded, &key(name))?;
        if loaded.is_truthy() {
            self.trace
                .push(format!("cache {name}: {}", loaded.type_name()));
            return Ok(loaded);
        }
        if self.remaining == 0 {
            self.trace
                .push(format!("budget exhausted before loader {name}"));
            return Err(LoadError::Budget);
        }
        self.remaining -= 1;
        *self.calls.entry(name).or_default() += 1;
        self.trace.push(format!("enter {name}"));
        let loader = self.loaders[name];
        (self.run_loader)(self, name, loader)
    }
    // ANCHOR_END: cycle_require_lookup

    pub fn publish(&mut self, name: Name, value: Value) -> Result<(), LoadError> {
        self.trace
            .push(format!("publish {name}: {}", value.type_name()));
        self.heap.table_set(self.loaded, key(name), value)?;
        Ok(())
    }

    pub fn exports(&mut self, name: Name, inner: Value) -> Result<Value, LoadError> {
        let table = self.heap.allocate_table()?;
        self.heap.table_set(table, key("name"), key(name))?;
        self.heap.table_set(
            table,
            key("answer"),
            Value::Number(if name == "B" { 7.0 } else { 42.0 }),
        )?;
        self.heap.table_set(table, key("inner"), inner)?;
        Ok(Value::Table(table))
    }

    pub fn field(&self, table: &Value, field: &str) -> Result<Value, LoadError> {
        let Value::Table(table) = table else {
            panic!("final export must be a table")
        };
        Ok(self.heap.table_get(*table, &key(field))?)
    }
}

fn self_loader(vm: &mut CycleVm, name: Name) -> Result<Value, LoadError> {
    let inner = vm.require(name)?;
    vm.exports(name, inner)
}
fn a_loader(vm: &mut CycleVm, name: Name) -> Result<Value, LoadError> {
    let b = vm.require("B")?;
    vm.exports(name, b)
}
fn b_loader(vm: &mut CycleVm, name: Name) -> Result<Value, LoadError> {
    let a = vm.require("A")?;
    vm.exports(name, a)
}
fn true_loader(_vm: &mut CycleVm, _name: Name) -> Result<Value, LoadError> {
    Ok(Value::Boolean(true))
}
fn partial_loader(vm: &mut CycleVm, name: Name) -> Result<Value, LoadError> {
    let exports = vm.exports(name, Value::Nil)?;
    let Value::Table(table) = exports else {
        unreachable!()
    };
    vm.heap
        .table_set(table, key("ready"), Value::Boolean(false))?;
    vm.publish(name, exports.clone())?;
    let reentered = vm.require(name)?;
    let saw_unready = vm.field(&reentered, "ready")? == Value::Boolean(false);
    vm.heap
        .table_set(table, key("saw_unready"), Value::Boolean(saw_unready))?;
    vm.heap
        .table_set(table, key("same"), Value::Boolean(reentered == exports))?;
    vm.heap
        .table_set(table, key("ready"), Value::Boolean(true))?;
    Ok(exports)
}
