use lua_app::{compile_and_disassemble, execute};
use lua_core::{ByteString, Value};

fn value(source: &str) -> Value {
    execute(source.as_bytes()).unwrap()
}

#[test]
fn expression_semantics_survive_the_vm_migration() {
    assert_eq!(value("-3 % 2"), Value::Number(1.0));
    assert_eq!(value("-2 ^ 2"), Value::Number(-4.0));
    assert_eq!(value("#'A\\000\\255'"), Value::Number(3.0));
    assert_eq!(value("1 == '1'"), Value::Boolean(false));
    assert_eq!(
        value("'answer=' .. 42"),
        Value::String(ByteString::new(&b"answer=42"[..]))
    );
}

#[test]
fn jumps_preserve_short_circuiting() {
    assert_eq!(value("false and missing"), Value::Boolean(false));
    assert_eq!(value("true or missing"), Value::Boolean(true));
    let bytecode = compile_and_disassemble(b"false and missing").unwrap();
    assert!(bytecode.contains("JumpIfFalse"));
    assert!(bytecode.contains("GetName"));
}

#[test]
fn malformed_input_is_a_compile_error() {
    let error = execute(b"1 + )").unwrap_err().to_string();
    assert!(error.contains("expected an expression"));
    assert!(error.starts_with("1:5:"));
}
