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
fn test_arithmetic_coerces_trimmed_numeric_strings() {
    let (state, _gc) = compile_and_run("return '2' + ' 3e0 '");
    assert_eq!(return_value(&state), Value::Number(5.0));

    let (state, _gc) = compile_and_run("return -'  10 ' + (' 10  ' % '2') + ('2' ^ ' 3e0 ')");
    assert_eq!(return_value(&state), Value::Number(-2.0));
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
fn test_and_expression_does_not_overwrite_left_local() {
    let (state, _gc) = compile_and_run(
        "local i = 'x1'; local v = 1; local a = {x1 = 1}; if i and v and a[i] == v then return 1 end return 0",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
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
    let (state, _gc) = compile_and_run("local x = 0; if true then x = 1 end; return x");
    assert_eq!(return_value(&state), Value::Number(1.0));

    let (state2, _gc2) =
        compile_and_run("local x = 0; if false then x = 1 else x = 2 end; return x");
    assert_eq!(return_value(&state2), Value::Number(2.0));
}

#[test]
fn test_block_locals_do_not_leak() {
    let (state, _gc) = compile_and_run("a = 7; do local a = 3 end; return a");
    assert_eq!(return_value(&state), Value::Number(7.0));

    let (state, _gc) =
        compile_and_run("a = 5; if true then local a = 9 else local a = 11 end; return a");
    assert_eq!(return_value(&state), Value::Number(5.0));
}

#[test]
fn test_break_inside_nested_non_breakable_block_exits_loop() {
    let (state, _gc) = compile_and_run(
        "local i = 0; while true do i = i + 1; do if i == 3 then break end end end; return i",
    );
    assert_eq!(return_value(&state), Value::Number(3.0));
}

#[test]
fn test_repeat_until_loops_until_condition_is_true() {
    let (state, _gc) = compile_and_run(
        "local i = 1; local c = 0; repeat c = c + 1; i = i + 1 until i > 3; return c * 10 + i",
    );
    assert_eq!(return_value(&state), Value::Number(34.0));

    let (state, _gc) = compile_and_run("local c = 0; repeat c = c + 1 until true; return c");
    assert_eq!(return_value(&state), Value::Number(1.0));
}

#[test]
fn test_numeric_for_loop() {
    let (state, _gc) =
        compile_and_run("local sum = 0; for i = 1, 5 do sum = sum + i end; return sum");
    assert_eq!(return_value(&state), Value::Number(15.0));

    let (state, _gc) =
        compile_and_run("local out = 0; for i = 5, 1, -2 do out = out * 10 + i end; return out");
    assert_eq!(return_value(&state), Value::Number(531.0));

    let (state, _gc) =
        compile_and_run("local count = 0; for i = 5, 1 do count = count + 1 end; return count");
    assert_eq!(return_value(&state), Value::Number(0.0));

    let (state, _gc) = compile_and_run("local i = 99; for i = 1, 3 do end; return i");
    assert_eq!(return_value(&state), Value::Number(99.0));
}

#[test]
fn test_tail_calls_reuse_the_current_lua_frame() {
    let (state, _gc) = compile_and_run(
        "function deep(n) if n > 0 then return deep(n - 1) else return 101 end end; return deep(30000)",
    );
    assert_eq!(return_value(&state), Value::Number(101.0));

    let (state, _gc) = compile_and_run(
        "local a = {}; function a:deep(n) if n > 0 then return self:deep(n - 1) else return 101 end end; return a:deep(30000)",
    );
    assert_eq!(return_value(&state), Value::Number(101.0));
}

#[test]
fn test_tail_calls_replace_the_current_closure_upvalues() {
    let source = "
        Y = function(le)
          local function a(f)
            return le(function(x) return f(f)(x) end)
          end
          return a(a)
        end
        F = function(f)
          return function(n)
            if n == 0 then return 1 else return n * f(n - 1) end
          end
        end
        local fat = Y(F)
        return fat(4)
    ";
    let (state, _gc) = compile_and_run(source);
    assert_eq!(return_value(&state), Value::Number(24.0));
}

#[test]
fn test_missing_lua_call_arguments_are_nil() {
    let (state, _gc) = compile_and_run(
        "local function keep(x) return x end; local stale = keep({value = 9}); local function f(a, b) if b == nil then return 1 end return 0 end; return f(42)",
    );
    assert_eq!(return_value(&state), Value::Number(1.0));
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

#[test]
fn test_table_constructor_array_fields() {
    let (state, _gc) = compile_and_run("local t = {10, 20, 30}; return t[2]");
    assert_eq!(return_value(&state), Value::Number(20.0));
}

#[test]
fn test_table_constructor_hash_fields() {
    let (state, _gc) = compile_and_run("local t = {answer = 42}; return t.answer");
    assert_eq!(return_value(&state), Value::Number(42.0));

    let (state2, _gc2) = compile_and_run("local key = 'x'; local t = {[key] = 9}; return t.x");
    assert_eq!(return_value(&state2), Value::Number(9.0));
}

#[test]
fn test_table_constructor_mixed_fields() {
    let (state, _gc) = compile_and_run(
        "local t = {10, 20, answer = 30, [4] = 40}; return t[1] + t[2] + t.answer + t[4]",
    );
    assert_eq!(return_value(&state), Value::Number(100.0));
}

#[test]
fn test_table_constructor_flushes_large_array_part() {
    let source = "local t = {\
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10,\
        11, 12, 13, 14, 15, 16, 17, 18, 19, 20,\
        21, 22, 23, 24, 25, 26, 27, 28, 29, 30,\
        31, 32, 33, 34, 35, 36, 37, 38, 39, 40,\
        41, 42, 43, 44, 45, 46, 47, 48, 49, 50,\
        51, 52, 53, 54, 55\
    }; return t[1] + t[50] + t[55]";
    let (state, _gc) = compile_and_run(source);
    assert_eq!(return_value(&state), Value::Number(106.0));
}
