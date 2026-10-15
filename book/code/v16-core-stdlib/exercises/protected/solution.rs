use crate::model::{error_text, Outcome};
use lua_core::Value;

// ANCHOR: protected_pcall_solution
pub fn pack_pcall(outcome: Outcome) -> Vec<Value> {
    match outcome {
        Ok(values) => {
            let mut results = vec![Value::Boolean(true)];
            results.extend(values);
            results
        }
        Err(error) => vec![Value::Boolean(false), error_text(error)],
    }
}
// ANCHOR_END: protected_pcall_solution

// ANCHOR: protected_xpcall_solution
pub fn pack_xpcall(outcome: Outcome, handler: &mut dyn FnMut(Value) -> Outcome) -> Vec<Value> {
    match outcome {
        Ok(values) => {
            let mut results = vec![Value::Boolean(true)];
            results.extend(values);
            results
        }
        Err(error) => {
            let handled = handler(error_text(error))
                .unwrap_or_else(|handler_error| vec![error_text(handler_error)]);
            vec![
                Value::Boolean(false),
                handled.into_iter().next().unwrap_or(Value::Nil),
            ]
        }
    }
}
// ANCHOR_END: protected_xpcall_solution
