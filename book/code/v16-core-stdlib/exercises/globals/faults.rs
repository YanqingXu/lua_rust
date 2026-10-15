use crate::model::Environment;
use lua_core::{Heap, HeapError};

pub fn install_snapshot(heap: &mut Heap) -> Result<Environment, HeapError> {
    Environment::snapshot(heap)
}
