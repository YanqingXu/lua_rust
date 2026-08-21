use lua_app::{compile_and_disassemble, execute};
use lua_core::Value;

fn value(source: &str) -> Value {
    execute(source.as_bytes()).unwrap()
}

#[test]
fn parallel_assignment_reads_all_old_values_first() {
    assert_eq!(
        value("local a, b = 10, 20\na, b = b, a\nreturn a * 100 + b"),
        Value::Number(2010.0)
    );
}

#[test]
fn block_shadowing_restores_the_outer_local() {
    assert_eq!(
        value(
            "local answer = 40\n\
             do\n\
               local answer = answer + 2\n\
               answer = 99\n\
             end\n\
             return answer"
        ),
        Value::Number(40.0)
    );
}

#[test]
fn undefined_globals_and_uninitialized_locals_are_nil() {
    assert_eq!(value("return missing"), Value::Nil);
    assert_eq!(value("local empty\nreturn empty"), Value::Nil);
}

#[test]
fn globals_and_locals_use_different_storage() {
    assert_eq!(
        value(
            "answer = 41\n\
             local answer = answer + 1\n\
             do local answer = 100 end\n\
             return answer"
        ),
        Value::Number(42.0)
    );
    let bytecode = compile_and_disassemble(b"answer = 42\nreturn answer").unwrap();
    assert!(bytecode.contains("SetGlobal"));
    assert!(bytecode.contains("GetGlobal"));
}

#[test]
fn missing_assignment_values_are_filled_with_nil() {
    assert_eq!(value("local first, second = 1\nreturn second"), Value::Nil);
}

#[test]
fn malformed_statement_is_a_compile_error() {
    let error = execute(b"local = 1").unwrap_err().to_string();
    assert!(error.contains("expected a variable name"));
    assert!(error.starts_with("1:7:"));
}
