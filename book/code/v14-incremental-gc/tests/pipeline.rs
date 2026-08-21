use lua_app::{compile_and_disassemble, execute, execute_multi, AppError};
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::{Vm, VmError, VmErrorKind};

fn value(source: &str) -> Value {
    execute(source.as_bytes()).unwrap()
}

fn runtime_error(source: &str) -> VmError {
    match execute(source.as_bytes()).unwrap_err() {
        AppError::Runtime(error) => error,
        other => panic!("expected runtime error, got {other}"),
    }
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

#[test]
fn counter_closure_keeps_and_mutates_a_closed_upvalue() {
    assert_eq!(
        value(
            "function make_counter(start)\n\
               local current = start\n\
               return function(step)\n\
                 current = current + step\n\
                 return current\n\
               end\n\
             end\n\
             local counter = make_counter(10)\n\
             local first = counter(2)\n\
             local second = counter(3)\n\
             return first * 100 + second"
        ),
        Value::Number(1_215.0)
    );
}

#[test]
fn sibling_closures_share_one_upvalue_cell() {
    assert_eq!(
        value(
            "function make_pair()\n\
               local current = 0\n\
               local function get() return current end\n\
               local function add(value) current = current + value return current end\n\
               return get, add\n\
             end\n\
             local get, add = make_pair()\n\
             local changed = add(4)\n\
             return changed * 100 + get()"
        ),
        Value::Number(404.0)
    );
}

#[test]
fn captures_are_forwarded_through_intermediate_functions() {
    assert_eq!(
        value(
            "function outer()\n\
               local answer = 42\n\
               return function()\n\
                 return function() return answer end\n\
               end\n\
             end\n\
             return outer()()()"
        ),
        Value::Number(42.0)
    );
}

#[test]
fn local_function_can_capture_itself_for_recursion() {
    assert_eq!(
        value(
            "local function factorial(n)\n\
               if n <= 1 then return 1 end\n\
               return n * factorial(n - 1)\n\
             end\n\
             return factorial(6)"
        ),
        Value::Number(720.0)
    );
}

#[test]
fn leaving_a_block_closes_before_register_reuse() {
    assert_eq!(
        value(
            "local saved\n\
             do\n\
               local captured = 10\n\
               saved = function() return captured end\n\
             end\n\
             local reused = 99\n\
             return saved()"
        ),
        Value::Number(10.0)
    );
    assert_eq!(
        value(
            "local saved\n\
             while true do\n\
               local captured = 41\n\
               saved = function() return captured end\n\
               break\n\
             end\n\
             local reused = 9\n\
             return saved()"
        ),
        Value::Number(41.0)
    );
}

#[test]
fn loop_iterations_close_captured_variables_independently() {
    assert_eq!(
        value(
            "local functions = {}\n\
             for index = 1, 3 do\n\
               functions[index] = function() return index end\n\
             end\n\
             return functions[1]() * 100 + functions[2]() * 10 + functions[3]()"
        ),
        Value::Number(123.0)
    );
    assert_eq!(
        value(
            "local functions = {}\n\
             for index, item in ipairs({10, 20, 30}) do\n\
               functions[index] = function() return index + item end\n\
             end\n\
             return functions[1]() + functions[2]() + functions[3]()"
        ),
        Value::Number(66.0)
    );
}

#[test]
fn closures_created_from_one_proto_still_have_distinct_identity() {
    assert_eq!(
        value(
            "function make() return function() return 1 end end\n\
             local first = make()\n\
             local second = make()\n\
             return first == second"
        ),
        Value::Boolean(false)
    );
}

#[test]
fn tail_call_closes_the_replaced_frame_before_continuing() {
    assert_eq!(
        value(
            "local saved\n\
             function finish() return saved() end\n\
             function start()\n\
               local captured = 77\n\
               saved = function() return captured end\n\
               return finish()\n\
             end\n\
             return start()"
        ),
        Value::Number(77.0)
    );
}

#[test]
fn index_accepts_table_and_function_metamethods() {
    assert_eq!(
        value(
            "local defaults = { answer = 42 }\n\
             local inherited = setmetatable({}, { __index = defaults })\n\
             local computed = setmetatable({}, {\n\
               __index = function(self, key) return key .. '-value' end\n\
             })\n\
             return inherited.answer * 10 + ((computed.name == 'name-value') and 1 or 0)"
        ),
        Value::Number(421.0)
    );
}

#[test]
fn newindex_can_redirect_to_a_table_or_function_and_raw_access_bypasses_it() {
    assert_eq!(
        value(
            "local table_target = {}\n\
             local table_proxy = setmetatable({}, { __newindex = table_target })\n\
             table_proxy.answer = 21\n\
             local function_target = {}\n\
             local function_proxy = setmetatable({}, {\n\
               __newindex = function(self, key, item) rawset(function_target, key, item * 2) end\n\
             })\n\
             function_proxy.answer = 21\n\
             rawset(function_proxy, 'raw', 7)\n\
             return table_target.answer * 100 + function_target.answer * 10\n\
               + rawget(function_proxy, 'raw')"
        ),
        Value::Number(2_527.0)
    );
}

#[test]
fn existing_keys_bypass_newindex_and_rawget_bypasses_index() {
    assert_eq!(
        value(
            "local redirected = {}\n\
             local object = setmetatable({ existing = 2 }, {\n\
               __index = function() return 99 end,\n\
               __newindex = function(self, key, item) redirected[key] = item end\n\
             })\n\
             object.existing = 7\n\
             object.missing = 8\n\
             return object.existing * 100 + redirected.missing * 10\n\
               + ((rawget(object, 'unknown') == nil) and 1 or 0)"
        ),
        Value::Number(781.0)
    );
}

#[test]
fn arithmetic_concat_length_call_and_tostring_use_metamethods() {
    assert_eq!(
        value(
            "local mt = {}\n\
             mt.__add = function(a, b) return a.value + b.value end\n\
             mt.__unm = function(a) return -a.value end\n\
             mt.__concat = function(a, b) return tostring(a.value) .. tostring(b.value) end\n\
             mt.__len = function(a) return a.value end\n\
             mt.__call = function(a, extra) return a.value + extra end\n\
             mt.__tostring = function(a) return 'box-' .. a.value end\n\
             local a = setmetatable({ value = 2 }, mt)\n\
             local b = setmetatable({ value = 3 }, mt)\n\
             return (a + b) * 10000 + (-a) * 1000 + #b * 100\n\
               + a(4) * 10 + (((a .. b) == '23' and tostring(a) == 'box-2') and 1 or 0)"
        ),
        Value::Number(48_361.0)
    );
}

#[test]
fn binary_operators_fall_back_to_the_right_operand_metamethod() {
    assert_eq!(
        value(
            "local right = setmetatable({ value = 7 }, {\n\
               __add = function(left, self) return left + self.value end\n\
             })\n\
             return 5 + right"
        ),
        Value::Number(12.0)
    );
}

#[test]
fn comparisons_require_a_shared_handler_and_le_falls_back_to_lt() {
    assert_eq!(
        value(
            "local mt = {}\n\
             mt.__lt = function(a, b) return a.value < b.value end\n\
             mt.__eq = function(a, b) return a.value == b.value end\n\
             local a = setmetatable({ value = 2 }, mt)\n\
             local b = setmetatable({ value = 3 }, mt)\n\
             local c = setmetatable({ value = 2 }, { __eq = mt.__eq, __lt = mt.__lt })\n\
             return ((a < b) and 1000 or 0) + ((a <= b) and 100 or 0)\n\
               + ((b > a) and 10 or 0) + ((a == c) and 1 or 0)"
        ),
        Value::Number(1_111.0)
    );
    let error = runtime_error(
        "local a = setmetatable({}, { __lt = function() return true end })\n\
         local b = setmetatable({}, { __lt = function() return true end })\n\
         return a < b",
    );
    assert_eq!(error.kind, VmErrorKind::Type);
    assert!(error.message.contains("cannot compare"));
}

#[test]
fn primitive_and_function_type_metatables_are_visible_to_language_code() {
    assert_eq!(
        value(
            "settypemetatable('number', {\n\
               __index = { double = function(self) return self * 2 end }\n\
             })\n\
             settypemetatable('function', { __index = { kind = 7 } })\n\
             local function item() end\n\
             return (21):double() * 10 + item.kind"
        ),
        Value::Number(427.0)
    );
}

#[test]
fn metatable_protection_controls_get_and_set() {
    assert_eq!(
        value(
            "local object = {}\n\
             setmetatable(object, { __metatable = 'locked' })\n\
             return getmetatable(object) == 'locked'"
        ),
        Value::Boolean(true)
    );
    let error = runtime_error(
        "local object = {}\n\
         setmetatable(object, { __metatable = 'locked' })\n\
         return setmetatable(object, {})",
    );
    assert_eq!(error.kind, VmErrorKind::Metamethod);
    assert!(error.message.contains("protected metatable"));
}

#[test]
fn metamethod_cycles_stop_at_a_classified_limit() {
    let index = runtime_error(
        "local left, right = {}, {}\n\
         setmetatable(left, { __index = right })\n\
         setmetatable(right, { __index = left })\n\
         return left.missing",
    );
    assert_eq!(index.kind, VmErrorKind::Limit);
    assert!(index.message.contains("__index chain limit"));

    let newindex = runtime_error(
        "local left, right = {}, {}\n\
         setmetatable(left, { __newindex = right })\n\
         setmetatable(right, { __newindex = left })\n\
         left.missing = 1",
    );
    assert_eq!(newindex.kind, VmErrorKind::Limit);
    assert!(newindex.message.contains("__newindex chain limit"));

    let call = runtime_error(
        "local left, right = {}, {}\n\
         setmetatable(left, { __call = right })\n\
         setmetatable(right, { __call = left })\n\
         return left()",
    );
    assert_eq!(call.kind, VmErrorKind::Limit);
    assert!(call.message.contains("__call chain limit"));
}

#[test]
fn tostring_metamethod_must_return_a_string() {
    let error = runtime_error(
        "local object = setmetatable({}, { __tostring = function() return 42 end })\n\
         return tostring(object)",
    );
    assert_eq!(error.kind, VmErrorKind::Metamethod);
    assert!(error.message.contains("must return a string"));
}

#[test]
fn full_gc_reclaims_an_unreachable_self_referential_table() {
    assert_eq!(
        value(
            "local before = gc_live()\n\
             local reclaimed\n\
             do\n\
               local cycle = {}\n\
               cycle.self = cycle\n\
             end\n\
             reclaimed = gc_collect()\n\
             return reclaimed >= 1 and gc_live() < before + 1"
        ),
        Value::Boolean(true)
    );
}

#[test]
fn stack_globals_closures_and_metatables_are_gc_roots() {
    assert_eq!(
        value(
            "global_root = { value = 5 }\n\
             local captured\n\
             do\n\
               local object = setmetatable({ value = 42 }, {\n\
                 __index = { bonus = 7 }\n\
               })\n\
               object.self = object\n\
               captured = function() return object.value + object.bonus end\n\
             end\n\
             gc_collect()\n\
             return captured() * 10 + global_root.value"
        ),
        Value::Number(495.0)
    );
}

#[test]
fn caller_frames_remain_roots_while_a_callee_collects() {
    assert_eq!(
        value(
            "function force_collection() return gc_collect() end\n\
             function outer()\n\
               local live = { answer = 42 }\n\
               force_collection()\n\
               return live.answer\n\
             end\n\
             return outer()"
        ),
        Value::Number(42.0)
    );
}

#[test]
fn weak_values_keys_and_ephemeron_values_follow_reachability() {
    assert_eq!(
        value(
            "local weak_values = setmetatable({}, { __mode = 'v' })\n\
             do weak_values.answer = {} end\n\
             gc_collect()\n\
             local values_gone = weak_values.answer == nil\n\
             local weak_keys = setmetatable({}, { __mode = 'k' })\n\
             do local key = {} weak_keys[key] = {} end\n\
             gc_collect()\n\
             local keys_gone = next(weak_keys) == nil\n\
             local live_key = {}\n\
             weak_keys[live_key] = { answer = 42 }\n\
             gc_collect()\n\
             return values_gone and keys_gone and weak_keys[live_key].answer == 42"
        ),
        Value::Boolean(true)
    );
}

#[test]
fn finalizers_run_in_object_id_order() {
    assert_eq!(
        value(
            "local log = {}\n\
             do\n\
               local first = newuserdata()\n\
               setfinalizer(first, function() log[#log + 1] = 1 end)\n\
               local second = newuserdata()\n\
               setfinalizer(second, function() log[#log + 1] = 2 end)\n\
             end\n\
             gc_collect()\n\
             return log[1] * 10 + log[2]"
        ),
        Value::Number(12.0)
    );
}

#[test]
fn resurrected_userdata_is_finalized_once_then_dies_later() {
    assert_eq!(
        value(
            "local runs = 0\n\
             local identity\n\
             do\n\
               local item = newuserdata()\n\
               identity = userdataid(item)\n\
               setfinalizer(item, function(value)\n\
                 runs = runs + 1\n\
                 rescued = value\n\
               end)\n\
             end\n\
             gc_collect()\n\
             local same = userdataid(rescued) == identity\n\
             local after_rescue = gc_live()\n\
             rescued = nil\n\
             gc_collect()\n\
             local after_second_death = gc_live()\n\
             gc_collect()\n\
             return same and runs == 1 and after_second_death < after_rescue"
        ),
        Value::Boolean(true)
    );
}

#[test]
fn allocator_threshold_advances_incremental_gc_at_vm_checkpoints() {
    let proto = compile(
        b"local sum = 0\n\
          for index = 1, 100 do\n\
            local item = { value = index }\n\
            sum = sum + item.value\n\
          end\n\
          return sum",
    )
    .unwrap();
    assert_eq!(
        Vm::new()
            .with_gc_allocation_threshold(1)
            .execute(&proto)
            .unwrap(),
        Value::Number(5_050.0)
    );
}

#[test]
fn one_failing_finalizer_does_not_skip_the_remaining_queue() {
    let first = compile(
        b"finalizer_after_error = false\n\
          do\n\
            local broken = newuserdata()\n\
            setfinalizer(broken, function() return {} + 1 end)\n\
            local later = newuserdata()\n\
            setfinalizer(later, function() finalizer_after_error = true end)\n\
          end\n\
          gc_collect()",
    )
    .unwrap();
    let mut vm = Vm::new();
    let error = vm.execute(&first).unwrap_err();
    assert_eq!(error.kind, VmErrorKind::Type);

    let probe = compile(b"return finalizer_after_error").unwrap();
    assert_eq!(vm.execute(&probe).unwrap(), Value::Boolean(true));
}

#[test]
fn a_finalizer_can_reenter_the_collector_without_running_twice() {
    assert_eq!(
        value(
            "local runs = 0\n\
             do\n\
               local item = newuserdata()\n\
               setfinalizer(item, function()\n\
                 runs = runs + 1\n\
                 gc_collect()\n\
               end)\n\
             end\n\
             gc_collect()\n\
             return runs"
        ),
        Value::Number(1.0)
    );
}

#[test]
fn explicit_gc_step_reports_completion_without_auto_step_stealing_it() {
    assert_eq!(
        value(
            "local rounds = 0\n\
             while not gc_step(1) do\n\
               rounds = rounds + 1\n\
               if rounds > 1000 then return false end\n\
             end\n\
             return rounds > 0 and gc_phase() == 'pause'"
        ),
        Value::Boolean(true)
    );
}
