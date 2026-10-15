//! 只模拟已经选好 loader、写入 loading=true 后的普通 Result 协议，不模拟 Heap/IO/GC。
use lua_core::Value;
use lua_vm::{VmError, VmErrorKind};
use std::collections::BTreeMap;

pub type Outcome = Result<Vec<Value>, VmError>;
pub type Loader = fn(&mut RetryVm, &str) -> Outcome;
pub type Attempt = fn(&mut RetryVm, &str, Loader) -> Result<Value, VmError>;

pub struct RetryVm {
    pub loaded: BTreeMap<String, Value>,
    pub calls: BTreeMap<String, usize>,
    pub side_effects: usize,
    attempt: Attempt,
}

impl RetryVm {
    pub fn new(attempt: Attempt) -> Self {
        Self {
            loaded: BTreeMap::from([("outer".to_owned(), Value::Number(900.0))]),
            calls: BTreeMap::new(),
            side_effects: 0,
            attempt,
        }
    }

    pub fn cached(&self, name: &str) -> Value {
        self.loaded.get(name).cloned().unwrap_or(Value::Nil)
    }

    pub fn require(&mut self, name: &str, loader: Loader) -> Result<Value, VmError> {
        let cached = self.cached(name);
        if cached.is_truthy() {
            return Ok(cached);
        }
        self.loaded.insert(name.to_owned(), Value::Boolean(true));
        (self.attempt)(self, name, loader)
    }

    pub fn finish(&mut self, name: &str, results: Vec<Value>) -> Value {
        let returned = results.into_iter().next().unwrap_or(Value::Nil);
        if returned != Value::Nil {
            self.loaded.insert(name.to_owned(), returned.clone());
            returned
        } else {
            let cached = self.cached(name);
            if cached == Value::Nil {
                Value::Boolean(true)
            } else {
                cached
            }
        }
    }

    pub fn entered(&mut self, name: &str) {
        *self.calls.entry(name.to_owned()).or_default() += 1;
        self.side_effects += 1;
    }
}

pub fn loader_error(name: &str) -> VmError {
    VmError {
        kind: VmErrorKind::Runtime,
        pc: 7,
        line: 3,
        message: format!("loader failed: {name}"),
        traceback: vec!["require caller".to_owned()],
    }
}

pub fn fail(vm: &mut RetryVm, name: &str) -> Outcome {
    vm.entered(name);
    Err(loader_error(name))
}

pub fn publish_then_fail(vm: &mut RetryVm, name: &str) -> Outcome {
    vm.entered(name);
    vm.loaded.insert(name.to_owned(), Value::Number(77.0));
    Err(loader_error(name))
}

pub fn repaired(vm: &mut RetryVm, name: &str) -> Outcome {
    vm.entered(name);
    Ok(vec![Value::Number(42.0), Value::Number(99.0)])
}

pub fn dependency(vm: &mut RetryVm, name: &str) -> Outcome {
    vm.entered(name);
    Ok(vec![Value::Number(41.0)])
}

pub fn depends_then_fails(vm: &mut RetryVm, name: &str) -> Outcome {
    vm.entered(name);
    assert_eq!(vm.require("dependency", dependency)?, Value::Number(41.0));
    Err(loader_error(name))
}

pub fn depends_then_succeeds(vm: &mut RetryVm, name: &str) -> Outcome {
    vm.entered(name);
    let value = vm.require("dependency", dependency)?;
    assert_eq!(value, Value::Number(41.0));
    Ok(vec![Value::Number(42.0)])
}
