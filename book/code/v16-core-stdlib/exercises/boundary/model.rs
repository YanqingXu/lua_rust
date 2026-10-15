//! A synchronous Result/depth model, not a VM continuation or GC implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryError {
    Ordinary(&'static str),
    YieldDenied,
    YieldSignal,
}

pub type Callback = fn(&mut BoundaryVm) -> Result<i64, BoundaryError>;
pub type BoundaryHook = fn(&mut BoundaryVm, Callback) -> Result<i64, BoundaryError>;

pub struct BoundaryVm {
    pub depth: usize,
    pub pending_values: Option<Vec<i64>>,
    pub pending_frames: Vec<&'static str>,
    pub trace: Vec<String>,
    hook: BoundaryHook,
}

impl BoundaryVm {
    pub fn new(hook: BoundaryHook, depth: usize) -> Self {
        Self {
            depth,
            pending_values: None,
            pending_frames: Vec::new(),
            trace: Vec::new(),
            hook,
        }
    }

    pub fn invoke(&mut self, callback: Callback) -> Result<i64, BoundaryError> {
        (self.hook)(self, callback)
    }

    pub fn record(&mut self, label: &str) {
        self.trace.push(format!("{label} depth={}", self.depth));
    }

    pub fn try_yield(&mut self, value: i64) -> Result<i64, BoundaryError> {
        if self.depth != 0 {
            self.record("yield rejected");
            return Err(BoundaryError::YieldDenied);
        }
        self.pending_values = Some(vec![value]);
        self.pending_frames.push("yield-frame");
        self.record("yield published");
        Err(BoundaryError::YieldSignal)
    }

    pub fn summary(&self, result: &Result<i64, BoundaryError>) -> String {
        format!(
            "result={result:?} depth={} pending={:?} frames={:?}",
            self.depth, self.pending_values, self.pending_frames
        )
    }
}

pub fn success(vm: &mut BoundaryVm) -> Result<i64, BoundaryError> {
    vm.record("callback success");
    Ok(42)
}

pub fn ordinary_error(vm: &mut BoundaryVm) -> Result<i64, BoundaryError> {
    vm.record("callback error");
    Err(BoundaryError::Ordinary("callback failed"))
}

pub fn yielding_callback(vm: &mut BoundaryVm) -> Result<i64, BoundaryError> {
    vm.try_yield(99)
}

// ANCHOR: boundary_nested_probe
pub fn nested_after_caught_error(vm: &mut BoundaryVm) -> Result<i64, BoundaryError> {
    vm.record("nested: before inner");
    let inner = vm.invoke(ordinary_error);
    if inner != Err(BoundaryError::Ordinary("callback failed")) {
        return Err(BoundaryError::Ordinary("unexpected nested result"));
    }
    vm.record("nested: after inner");
    vm.try_yield(99) // Still inside the outer boundary: this must not publish pending state.
}
// ANCHOR_END: boundary_nested_probe
