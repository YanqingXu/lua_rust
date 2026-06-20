//! VM 集成测试 — 覆盖从编译到执行的完整管线
//!
//! 测试 Lua 源码 → Parser → CodeGenerator → VM execute 的端到端行为。

use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::table::Table;
use lua_core::value::Value;
use lua_vm::execute::execute_proto;
use lua_vm::state::LuaState;

/// 辅助函数：编译并执行 Lua 源码
fn compile_and_run(source: &str) -> (LuaState, GarbageCollector) {
    let mut gc = GarbageCollector::new();
    let global_table = gc.create_root(Table::new());
    let mut state = LuaState::with_global_table(global_table);

    let mut parser = Parser::new(source);
    let chunk = parser.parse().expect("Parse should succeed");
    let cg = CodeGenerator::new(&mut gc);
    let proto = cg
        .generate(&chunk, "<test>")
        .expect("Codegen should succeed");

    let _ = execute_proto(&mut state, &proto, &mut gc);
    (state, gc)
}

/// 执行后检查返回值（返回值被放置在调用帧的 func 索引位置）
fn return_value(state: &LuaState) -> Value {
    // After execute_proto returns, the result is at the initial call frame's func index
    // which is typically 0 (the first stack slot)
    state.stack.at(0).cloned().unwrap_or(Value::Nil)
}

#[test]
fn test_simple_return_number() {
    let (state, _gc) = compile_and_run("return 42");
    assert_eq!(return_value(&state), Value::Number(42.0));
}

#[test]
fn test_return_nil() {
    let (state, _gc) = compile_and_run("return nil");
    assert_eq!(return_value(&state), Value::Nil);
}

#[test]
fn test_return_boolean() {
    let (state, _gc) = compile_and_run("return true");
    assert_eq!(return_value(&state), Value::Boolean(true));

    let (state2, _gc2) = compile_and_run("return false");
    assert_eq!(return_value(&state2), Value::Boolean(false));
}

#[test]
fn test_arithmetic_add() {
    let (state, _gc) = compile_and_run("return 10 + 20");
    assert_eq!(return_value(&state), Value::Number(30.0));
}

#[test]
fn test_arithmetic_sub_mul() {
    let (state, _gc) = compile_and_run("return 100 - 30");
    assert_eq!(return_value(&state), Value::Number(70.0));

    let (state2, _gc2) = compile_and_run("return 6 * 7");
    assert_eq!(return_value(&state2), Value::Number(42.0));
}

#[test]
fn test_arithmetic_div_mod() {
    let (state, _gc) = compile_and_run("return 10 / 3");
    let v = return_value(&state);
    if let Value::Number(n) = v {
        assert!((n - 3.3333333).abs() < 0.001);
    } else {
        panic!("Expected number, got {:?}", v);
    }
}

#[test]
fn test_local_variables() {
    let (state, _gc) = compile_and_run("local a = 5; local b = 7; return a + b");
    assert_eq!(return_value(&state), Value::Number(12.0));
}

#[test]
fn test_string_return() {
    let (state, _gc) = compile_and_run("return 'hello'");
    // Strings are returned as GcRef<GcString>. Verify it's a string value.
    assert!(matches!(return_value(&state), Value::String(_)));
}

#[test]
fn test_multiple_statements() {
    let (state, _gc) = compile_and_run("local x = 1; x = x + 1; return x");
    assert_eq!(return_value(&state), Value::Number(2.0));
}

#[test]
fn test_not_operator() {
    let (state, _gc) = compile_and_run("return not false");
    assert_eq!(return_value(&state), Value::Boolean(true));

    let (state2, _gc2) = compile_and_run("return not 42");
    assert_eq!(return_value(&state2), Value::Boolean(false));
}

#[test]
fn test_unary_minus() {
    let (state, _gc) = compile_and_run("return -15");
    assert_eq!(return_value(&state), Value::Number(-15.0));
}

#[test]
fn test_comparison_eq() {
    let (state, _gc) = compile_and_run("return 1 == 1");
    assert_eq!(return_value(&state), Value::Boolean(true));

    let (state2, _gc2) = compile_and_run("return 1 == 2");
    assert_eq!(return_value(&state2), Value::Boolean(false));
}

#[test]
fn test_comparison_lt() {
    let (state, _gc) = compile_and_run("return 1 < 2");
    assert_eq!(return_value(&state), Value::Boolean(true));

    let (state2, _gc2) = compile_and_run("return 2 < 1");
    assert_eq!(return_value(&state2), Value::Boolean(false));
}

#[test]
fn test_if_statement() {
    let (state, _gc) = compile_and_run(
        "local x = 0; if true then x = 1 end; return x",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state2, _gc2) = compile_and_run(
        "local x = 0; if false then x = 1 else x = 2 end; return x",
    );
    assert_eq!(return_value(&state2), Value::Number(2.0));
}

#[test]
fn test_concat() {
    let (state, _gc) = compile_and_run("return 'hello' .. 'world'");
    // Should produce a string value
    assert!(matches!(return_value(&state), Value::String(_)));
}

#[test]
fn test_len_operator() {
    let (state, _gc) = compile_and_run("return #''");
    assert_eq!(return_value(&state), Value::Number(0.0));
}
