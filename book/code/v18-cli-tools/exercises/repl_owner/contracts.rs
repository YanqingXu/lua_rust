use crate::chunks::{number, setup_values, Submit, ADVANCE, OBSERVE, SETUP};
use lua_app::Session;
use lua_core::Value;
use std::fmt::Debug;

pub fn equal<T: Debug + PartialEq>(label: &str, actual: T, expected: T) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label}: expected {expected:?}, got {actual:?}"))
    }
}

// ANCHOR: repl_owner_contract
pub fn submissions_preserve_globals_tables_closures_and_owner_edges(
    submit: Submit,
) -> Result<[Vec<Value>; 3], String> {
    let mut session = Session::new();
    let setup = submit(&mut session, SETUP).map_err(|error| error.to_string())?;
    equal("setup actual fields", setup.clone(), setup_values())?;
    let observed = submit(&mut session, OBSERVE).map_err(|error| error.to_string())?;
    let mut expected = [10, 40, 4, 7, 1, 9].map(number).to_vec();
    expected.push(Value::Nil); // A chunk local is not a REPL-wide binding.
    equal("next chunk after GC", observed.clone(), expected)?;
    let advanced = submit(&mut session, ADVANCE).map_err(|error| error.to_string())?;
    equal(
        "mutations persist in the same graph",
        advanced.clone(),
        vec![
            number(11),
            number(42),
            number(5),
            Value::Boolean(true),
            number(8),
            number(1),
            number(10),
            Value::Nil,
        ],
    )?;
    Ok([setup, observed, advanced])
}
// ANCHOR_END: repl_owner_contract
