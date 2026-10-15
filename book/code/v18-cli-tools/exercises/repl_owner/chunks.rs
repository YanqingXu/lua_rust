use lua_app::{AppError, Session};
use lua_core::Value;

pub type Submit = fn(&mut Session, &[u8]) -> Result<Vec<Value>, AppError>;

// ANCHOR: repl_owner_chunks
pub const SETUP: &[u8] = br#"
repl_number, repl_table = 10, { answer = 40 }
do
  local cell = { n = 2 }
  repl_next = function() cell.n = cell.n + 1; return cell.n end
end
repl_loads = 0
package.preload.repl_owner_mod = function()
  repl_loads = repl_loads + 1
  return { answer = 7 }
end
local module = require('repl_owner_mod')
debug.getregistry().repl_owner_cell = { answer = 9 }
local chunk_only = 123
return repl_number, repl_table.answer, repl_next(), module.answer,
  repl_loads, debug.getregistry().repl_owner_cell.answer, chunk_only
"#;

pub const OBSERVE: &[u8] = br#"
gc_collect()
return repl_number, repl_table and repl_table.answer, repl_next and repl_next(),
  package.loaded.repl_owner_mod and package.loaded.repl_owner_mod.answer,
  repl_loads, debug.getregistry().repl_owner_cell and
  debug.getregistry().repl_owner_cell.answer, chunk_only
"#;
// ANCHOR_END: repl_owner_chunks

pub const ADVANCE: &[u8] = br#"
repl_number = repl_number + 1
repl_table.answer = repl_table.answer + 2
local cached = require('repl_owner_mod')
cached.answer = cached.answer + 1
debug.getregistry().repl_owner_cell.answer = 10
gc_collect()
return repl_number, repl_table.answer, repl_next(),
  require('repl_owner_mod') == cached, require('repl_owner_mod').answer,
  repl_loads, debug.getregistry().repl_owner_cell.answer, chunk_only
"#;

pub fn number(value: i32) -> Value {
    Value::Number(value as f64)
}

pub fn setup_values() -> Vec<Value> {
    [10, 40, 3, 7, 1, 9, 123].map(number).to_vec()
}

pub fn observe_pair(submit: Submit) -> Result<[Vec<Value>; 2], String> {
    let mut session = Session::new();
    let first = submit(&mut session, SETUP).map_err(|error| error.to_string())?;
    let second = submit(&mut session, OBSERVE).map_err(|error| error.to_string())?;
    Ok([first, second])
}
