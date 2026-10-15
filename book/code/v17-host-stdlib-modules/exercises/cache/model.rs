//! Success-path experiment using real Value, without a Heap or module resolver.
use std::collections::BTreeMap;

use lua_core::Value;

#[derive(Default)]
pub struct Cache(BTreeMap<Vec<u8>, Value>);

impl Cache {
    pub fn get(&self, name: &[u8]) -> Value {
        self.0.get(name).cloned().unwrap_or(Value::Nil)
    }

    pub fn set(&mut self, name: &[u8], value: Value) {
        if value == Value::Nil {
            self.0.remove(name);
        } else {
            self.0.insert(name.to_vec(), value);
        }
    }
}

pub type Lookup = fn(&Cache, &[u8]) -> Option<Value>;
pub type Finish = fn(&mut Cache, &[u8], Vec<Value>) -> Value;

#[derive(Clone, Copy)]
pub struct Design {
    pub lookup: Lookup,
    pub finish: Finish,
}

pub struct Loader {
    /// None leaves the slot alone; Some(Nil) explicitly removes it.
    pub publish: Option<Value>,
    pub results: Vec<Value>,
}

pub struct ModuleVm {
    pub loaded: Cache,
    pub calls: usize,
    pub entry_slots: Vec<Value>,
    design: Design,
}

impl ModuleVm {
    pub fn new(design: Design) -> Self {
        Self {
            loaded: Cache::default(),
            calls: 0,
            entry_slots: Vec::new(),
            design,
        }
    }

    // ANCHOR: require_cache_driver
    pub fn require(&mut self, name: &[u8], loader: &Loader) -> Value {
        if let Some(hit) = (self.design.lookup)(&self.loaded, name) {
            return hit;
        }
        // Resolution is absent from this model. The real VM resolves first.
        self.loaded.set(name, Value::Boolean(true));
        self.calls += 1;
        self.entry_slots.push(self.loaded.get(name));
        if let Some(published) = &loader.publish {
            self.loaded.set(name, published.clone());
        }
        (self.design.finish)(&mut self.loaded, name, loader.results.clone())
    }
    // ANCHOR_END: require_cache_driver
}
