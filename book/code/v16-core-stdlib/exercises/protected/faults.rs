use crate::model::{error_text, Outcome};
use lua_core::Value;

pub fn first_result_only(outcome: Outcome) -> Vec<Value> {
    match outcome {
        Ok(values) => vec![
            Value::Boolean(true),
            values.into_iter().next().unwrap_or(Value::Nil),
        ],
        Err(error) => vec![Value::Boolean(false), error_text(error)],
    }
}

pub fn always_handle_and_forward_all(
    outcome: Outcome,
    handler: &mut dyn FnMut(Value) -> Outcome,
) -> Vec<Value> {
    let message = match outcome {
        Ok(values) => values.into_iter().next().unwrap_or(Value::Nil),
        Err(error) => error_text(error),
    };
    let handled = handler(message).unwrap_or_else(|error| vec![error_text(error)]);
    let mut results = vec![Value::Boolean(false)];
    results.extend(handled);
    results
}
