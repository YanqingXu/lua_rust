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

#[test]
fn if_elseif_else_executes_exactly_one_branch() {
    assert_eq!(
        value(
            "local answer = 0\n\
             if false then answer = 1\n\
             elseif 2 < 3 then answer = 42\n\
             else answer = 99 end\n\
             return answer"
        ),
        Value::Number(42.0)
    );
}

#[test]
fn while_checks_before_the_body_and_break_exits_it() {
    assert_eq!(
        value(
            "local i, sum = 0, 0\n\
             while i < 10 do\n\
               i = i + 1\n\
               if i == 4 then break end\n\
               sum = sum + i\n\
             end\n\
             return sum"
        ),
        Value::Number(6.0)
    );
    assert_eq!(
        value("while false do return 99 end\nreturn 1"),
        Value::Number(1.0)
    );
}

#[test]
fn repeat_checks_after_the_body_and_condition_sees_body_locals() {
    assert_eq!(
        value("repeat local finished = true until finished\nreturn 1"),
        Value::Number(1.0)
    );
}

#[test]
fn numeric_for_supports_positive_and_negative_steps() {
    assert_eq!(
        value("local sum = 0\nfor i = 1, 5 do sum = sum + i end\nreturn sum"),
        Value::Number(15.0)
    );
    assert_eq!(
        value("local sum = 0\nfor i = 5, 1, -2 do sum = sum + i end\nreturn sum"),
        Value::Number(9.0)
    );
}

#[test]
fn break_targets_the_nearest_loop() {
    assert_eq!(
        value(
            "local total = 0\n\
             for i = 1, 3 do\n\
               for j = 1, 3 do total = total + 1 break end\n\
             end\n\
             return total"
        ),
        Value::Number(3.0)
    );
}

#[test]
fn invalid_break_and_zero_step_are_structured_errors() {
    assert!(execute(b"break")
        .unwrap_err()
        .to_string()
        .contains("only valid inside a loop"));
    assert!(execute(b"for i = 1, 3, 0 do end")
        .unwrap_err()
        .to_string()
        .contains("step must not be zero"));
}
