use crate::model::{key, Environment};
use lua_core::{Heap, HeapError, Value};

// ANCHOR: globals_install_solution
pub fn install(heap: &mut Heap) -> Result<Environment, HeapError> {
    let table = heap.allocate_table()?;
    heap.table_set(table, key("seed"), Value::Number(1.0))?;
    heap.table_set(table, key("_G"), Value::Table(table))?;
    Ok(Environment::from_shared_table(table))
}
// ANCHOR_END: globals_install_solution
