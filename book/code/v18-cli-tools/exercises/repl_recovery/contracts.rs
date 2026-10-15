use crate::model::{Event, Repl, Submit};
use lua_app::AppError;
use lua_core::Value;

pub fn values(event: Event, expected: &[Value]) {
    match event {
        Event::Values(actual) => assert_eq!(actual, expected),
        other => panic!("expected values {expected:?}, got {}", other.describe()),
    }
}

// ANCHOR: repl_recovery_contract
pub fn source_lifecycle_recovers_after_errors(submit: Submit) {
    let mut repl = Repl::new(submit);
    assert!(matches!(repl.push("  "), Event::Blank));
    assert!(repl.source.is_empty());
    values(repl.push("saved=40; effects=0"), &[]);
    assert!(matches!(repl.push("if true then"), Event::Incomplete(_)));
    assert_eq!(repl.source, "if true then\n");
    assert!(matches!(repl.push(""), Event::Incomplete(_)));
    assert_eq!(repl.source, "if true then\n\n");
    assert!(matches!(repl.push("saved=saved+2"), Event::Incomplete(_)));
    values(repl.push("end"), &[]);
    assert!(repl.source.is_empty());
    assert!(matches!(
        repl.push("local ="),
        Event::Error(AppError::Compile(_))
    ));
    assert!(repl.source.is_empty());
    values(repl.push("=saved"), &[Value::Number(42.0)]);
    assert!(matches!(
        repl.push("effects=effects+1; error('recover')"),
        Event::Error(AppError::Runtime(_))
    ));
    assert!(repl.source.is_empty());
    values(
        repl.push("=effects,saved"),
        &[Value::Number(1.0), Value::Number(42.0)],
    );
    values(repl.push("return nil"), &[Value::Nil]);
    values(repl.push("return"), &[]);
    assert!(matches!(repl.push("if true then"), Event::Incomplete(_)));
    assert!(matches!(
        repl.push("=42"),
        Event::Error(AppError::Compile(_))
    ));
    assert!(repl.source.is_empty()); // pending 中 = 不改写，普通错误后恢复。
    values(repl.push("  =saved+1"), &[Value::Number(43.0)]);
}
// ANCHOR_END: repl_recovery_contract
