use lua_app::{compile_and_disassemble, execute, execute_multi};
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

#[test]
fn constructors_support_array_hash_and_mixed_fields() {
    assert_eq!(
        value(
            "local t = {10, 20, name = 'lua', [5] = 50}\n\
             return t[1] + t[2] + t[5] + #t"
        ),
        Value::Number(82.0)
    );
    assert_eq!(
        value("local t = {name = 'lua'}\nreturn t.name"),
        Value::String(lua_core::ByteString::new(&b"lua"[..]))
    );
}

#[test]
fn aliases_observe_updates_and_equality_uses_identity() {
    assert_eq!(
        value("local first = {}\nlocal second = first\nsecond.answer = 42\nreturn first.answer"),
        Value::Number(42.0)
    );
    assert_eq!(
        value("local first = {}\nlocal second = first\nreturn first == second"),
        Value::Boolean(true)
    );
    assert_eq!(value("return {} == {}"), Value::Boolean(false));
}

#[test]
fn writing_nil_deletes_and_missing_keys_read_nil() {
    assert_eq!(
        value("local t = {answer = 42}\nt.answer = nil\nreturn t.answer"),
        Value::Nil
    );
    assert_eq!(value("local t = {}\nreturn t.missing"), Value::Nil);
}

#[test]
fn table_assignment_addresses_are_captured_before_writes() {
    assert_eq!(
        value("local t = {{}, {}}\nlocal i = 1\nt[i], i = 42, 2\nreturn t[1]"),
        Value::Number(42.0)
    );
}

#[test]
fn self_reference_is_observable() {
    assert_eq!(
        value("local t = {}\nt.self = t\nreturn t.self == t"),
        Value::Boolean(true)
    );
}

#[test]
fn nil_nan_and_non_table_indexing_are_errors() {
    assert!(execute(b"local t = {}\nt[nil] = 1")
        .unwrap_err()
        .to_string()
        .contains("index is nil"));
    assert!(execute(b"local t = {}\nt[0 / 0] = 1")
        .unwrap_err()
        .to_string()
        .contains("index is NaN"));
    assert!(execute(b"local n = 1\nreturn n.x")
        .unwrap_err()
        .to_string()
        .contains("index a number value"));
}

#[test]
fn global_functions_and_recursion_work() {
    assert_eq!(
        value(
            "function factorial(n)\n\
               if n <= 1 then return 1 end\n\
               return n * factorial(n - 1)\n\
             end\n\
             return factorial(6)"
        ),
        Value::Number(720.0)
    );
}

#[test]
fn function_expressions_local_functions_and_nested_calls_work() {
    assert_eq!(
        value(
            "local double = function(x) return x * 2 end\n\
             local function add(left, right) return left + right end\n\
             return add(double(10), double(11))"
        ),
        Value::Number(42.0)
    );
}

#[test]
fn nested_functions_work_without_capture() {
    assert_eq!(
        value(
            "local outer = function(x)\n\
               local inner = function(y) return y + 1 end\n\
               return inner(x)\n\
             end\n\
             return outer(41)"
        ),
        Value::Number(42.0)
    );
}

#[test]
fn missing_parameters_are_nil_and_extra_arguments_are_ignored() {
    assert_eq!(
        value("function second(a, b) return b end\nreturn second(1)"),
        Value::Nil
    );
    assert_eq!(
        value("function first(a) return a end\nreturn first(42, 99)"),
        Value::Number(42.0)
    );
}

#[test]
fn methods_insert_self_and_evaluate_the_receiver_once() {
    assert_eq!(
        value(
            "local object = {base = 40}\n\
             function object:add(delta) return self.base + delta end\n\
             return object:add(2)"
        ),
        Value::Number(42.0)
    );
    assert_eq!(
        value(
            "calls = 0\n\
             object = {base = 40}\n\
             function object:add(delta) return self.base + delta end\n\
             function get_object() calls = calls + 1 return object end\n\
             return get_object():add(2) + calls"
        ),
        Value::Number(43.0)
    );
}

#[test]
fn function_equality_uses_identity() {
    assert_eq!(
        value("local f = function() return 1 end\nlocal g = f\nreturn f == g"),
        Value::Boolean(true)
    );
    assert_eq!(
        value("return function() end == function() end"),
        Value::Boolean(false)
    );
}

#[test]
fn invalid_calls_depth_limit_and_traceback_are_errors() {
    assert!(execute(b"local value = 1\nreturn value()")
        .unwrap_err()
        .to_string()
        .contains("call a number value"));
    let depth = execute(b"function forever() return 1 + forever() end\nreturn forever()")
        .unwrap_err()
        .to_string();
    assert!(depth.contains("call depth limit exceeded"));
    assert!(depth.contains("called from line"));

    let trace = execute(
        b"function inner() return 1 + {} end\n\
          function outer() local value = inner() return value end\n\
          return outer()",
    )
    .unwrap_err()
    .to_string();
    assert!(trace.contains("arithmetic needs numbers"));
    assert!(trace.matches("called from line").count() >= 1);
}

#[test]
fn multi_return_expands_only_in_the_final_list_position() {
    let results = execute_multi(
        b"function values() return 10, 20, 30 end\n\
          local a, b, c = values()\n\
          local d, e = 1, values()\n\
          local f, g, h = (values())\n\
          return a + b + c + d + e + f, g, h",
    )
    .unwrap();
    assert_eq!(results, vec![Value::Number(81.0), Value::Nil, Value::Nil]);
}

#[test]
fn calls_returns_and_table_constructors_share_the_open_result_protocol() {
    assert_eq!(
        value(
            "function values() return 10, 20, 30 end\n\
             function sum3(a, b, c) return a + b + c end\n\
             return sum3(1, values())"
        ),
        Value::Number(31.0)
    );
    assert_eq!(
        execute_multi(b"function values() return 10, 20, 30 end\nreturn 1, values()").unwrap(),
        vec![
            Value::Number(1.0),
            Value::Number(10.0),
            Value::Number(20.0),
            Value::Number(30.0),
        ]
    );
    assert_eq!(
        value(
            "function values() return 10, 20, 30 end\n\
             local t = { 1, values() }\n\
             return #t * 1000 + t[2] * 100 + t[3] * 10 + t[4]"
        ),
        Value::Number(5_230.0)
    );
}

#[test]
fn vararg_can_be_sliced_forwarded_and_returned() {
    assert_eq!(
        execute_multi(
            b"function collect(first, ...)\n\
                local a, b, c = ...\n\
                return first, a, b, c\n\
              end\n\
              function forward(...) return collect(...) end\n\
              return forward(1, 2, 3, 4)",
        )
        .unwrap(),
        vec![
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Number(3.0),
            Value::Number(4.0),
        ]
    );
    let error = execute(b"return ...").unwrap_err().to_string();
    assert!(error.contains("outside a vararg function"));
}

#[test]
fn tail_calls_reuse_the_lua_frame() {
    assert_eq!(
        value(
            "function countdown(n, total)\n\
               if n == 0 then return total end\n\
               return countdown(n - 1, total + 1)\n\
             end\n\
             return countdown(5000, 0)"
        ),
        Value::Number(5_000.0)
    );
}

#[test]
fn generic_for_accepts_builtin_and_lua_iterators() {
    assert_eq!(
        value(
            "local t = { 3, 4, x = 5 }\n\
             local sum, count = 0, 0\n\
             for key, item in pairs(t) do\n\
               sum = sum + item\n\
               count = count + 1\n\
             end\n\
             return sum * 10 + count"
        ),
        Value::Number(123.0)
    );
    assert_eq!(
        value(
            "local t = { 10, 20, nil, 40 }\n\
             local sum = 0\n\
             for index, item in ipairs(t) do sum = sum + item end\n\
             return sum"
        ),
        Value::Number(30.0)
    );
    assert_eq!(
        value(
            "function iter(limit, control)\n\
               control = control + 1\n\
               if control <= limit then return control, control * 2 end\n\
             end\n\
             local sum = 0\n\
             for key, item in iter, 3, 0 do sum = sum + key + item end\n\
             return sum"
        ),
        Value::Number(18.0)
    );
}

#[test]
fn iterator_contract_errors_are_structured() {
    assert!(execute(b"return pairs(1)")
        .unwrap_err()
        .to_string()
        .contains("pairs expected table"));
    assert!(execute(b"local t = { 1 }\nreturn next(t, 99)")
        .unwrap_err()
        .to_string()
        .contains("invalid key to 'next'"));
}
