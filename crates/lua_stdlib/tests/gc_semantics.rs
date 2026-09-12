#![forbid(unsafe_code)]

use lua_compiler::{codegen::CodeGenerator, parser::Parser};
use lua_core::{table::Table, value::Value};
use lua_vm::Runtime;

fn run(source: &str) -> Vec<Value> {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    let proto = {
        let chunk = Parser::new(source).parse().unwrap();
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, strings) = parts.split_mut();
        let proto = CodeGenerator::new_with_pool(gc, strings)
            .generate(&chunk, "<gc-semantics>")
            .unwrap();
        gc.create(proto)
    };
    let mut values = Vec::new();
    runtime
        .execute_proto_with_args(proto, Vec::new(), |_, returned| values = returned.to_vec())
        .unwrap();
    assert!(
        values
            .iter()
            .all(|value| matches!(value, Value::Nil | Value::Number(_) | Value::Boolean(_)))
    );
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
    values
}

#[test]
fn automatic_collection_starts_native_finalizers_without_inheriting_yield() {
    let values = run(r##"
        collectgarbage("setpause", 200)
        collectgarbage("setstepmul", 10000)
        local weak = setmetatable({}, {__mode = "v"})
        local value = newproxy(true)
        getmetatable(value).__gc = type
        weak[1] = value
        value = nil
        local attempts = 0
        repeat
            attempts = attempts + 1
            local garbage = {attempts, tostring(attempts), {}}
        until weak[1] == nil or attempts == 5000
        assert(weak[1] == nil, "allocation GC did not reach native finalizer")
        return true
    "##);
    assert_eq!(values, vec![Value::Boolean(true)]);
}

#[test]
fn real_finalizer_yield_remains_an_error_and_the_caller_can_continue() {
    let values = run(r##"
        collectgarbage("stop")
        local value = newproxy(true)
        getmetatable(value).__gc = coroutine.yield
        value = nil
        local ok, message = pcall(collectgarbage, "collect")
        assert(not ok and type(message) == "string")
        assert(string.find(message, "yield", 1, true))
        collectgarbage("collect")
        local co = coroutine.create(function() coroutine.yield(17); return 23 end)
        local resumed, first = coroutine.resume(co)
        assert(resumed and first == 17)
        local completed, second = coroutine.resume(co)
        assert(completed and second == 23)
        return true
    "##);
    assert_eq!(values, vec![Value::Boolean(true)]);
}

#[test]
fn native_top_changes_preserve_large_active_call_windows() {
    // Most registers are needed later in this same frame. setmetatable's native
    // result truncation must not shrink the initialized frame scanned at GC.
    let values = run(r##"
        local first = setmetatable({}, {})
        collectgarbage("collect")
        local a1,a2,a3,a4,a5,a6,a7,a8,a9,a10 = {},{},{},{},{},{},{},{},{},{}
        local b1,b2,b3,b4,b5,b6,b7,b8,b9,b10 = {},{},{},{},{},{},{},{},{},{}
        local c1,c2,c3,c4,c5,c6,c7,c8,c9,c10 = {},{},{},{},{},{},{},{},{},{}
        local kept = {first,a1,a2,a3,a4,a5,a6,a7,a8,a9,a10,
            b1,b2,b3,b4,b5,b6,b7,b8,b9,b10,c1,c2,c3,c4,c5,c6,c7,c8,c9,c10}
        assert(select("#", setmetatable(first, nil)) == 1)
        local ok, message, extra = pcall(error, "truncated native stack", 0)
        assert(not ok and message == "truncated native stack" and extra == nil)
        collectgarbage("collect")
        assert(#kept == 31 and kept[1] == first and kept[31] == c10)
        local function identity(...) return ... end
        assert(select("#", pcall(identity, 11, nil, 33)) == 4)
        return true
    "##);
    assert_eq!(values, vec![Value::Boolean(true)]);
}

#[test]
fn native_collection_does_not_keep_out_of_scope_weak_keys_or_values() {
    let values = run(r##"
        local weak = setmetatable({}, {__mode = "vk"})
        local x, y, z = {}, {}, {}
        weak[1], weak[2], weak[3] = x, y, z
        weak[string.rep("$", 11)] = string.rep("$", 11)
        for i = 4, 15 do weak[i] = {} end
        for i = 1, 15 do weak[{}] = i end
        for i = 1, 15 do local value = {}; weak[value] = value end
        collectgarbage("collect")
        local count = 0
        for key, value in pairs(weak) do
            assert((key == 1 and value == x) or (key == 2 and value == y) or
                (key == 3 and value == z) or key == value)
            count = count + 1
        end
        assert(count == 4)
        x, y, z = nil
        collectgarbage("collect")
        assert(next(weak) == string.rep("$", 11))
        assert(next(weak, string.rep("$", 11)) == nil)
        return true
        "##);
    assert_eq!(values, vec![Value::Boolean(true)]);
}

#[test]
fn logical_top_keeps_high_register_roots_without_changing_push_pop_results() {
    let mut runtime = Runtime::new();
    let high = {
        let mut parts = runtime.parts_mut().unwrap();
        let (state, gc, _) = parts.split_mut();
        let high = gc.create(Table::new());
        state.stack.set_top(64);
        state.current_call_info_mut().top = 64;
        *state.stack.at_mut(60).unwrap() = Value::Table(high);
        state.top = 3;
        state.set_top(1);
        assert_eq!(state.get_top(), 1);
        assert_eq!(state.stack.size(), 64);
        state.push_number(42.0);
        assert_eq!(state.get_top(), 2);
        assert_eq!(state.pop(), Some(Value::Number(42.0)));
        assert_eq!(state.get_top(), 1);
        high
    };
    runtime.collect_garbage_full().unwrap();
    {
        let mut parts = runtime.parts_mut().unwrap();
        let (state, gc, _) = parts.split_mut();
        assert!(gc.contains_registered(high));
        assert_eq!(state.stack.at(60), Some(&Value::Table(high)));
        *state.stack.at_mut(60).unwrap() = Value::Nil;
    }
    runtime.collect_garbage_full().unwrap();
    {
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, _) = parts.split_mut();
        assert!(!gc.contains_registered(high));
    }
    assert_eq!(runtime.close().unwrap().remaining_objects, 0);
}

#[test]
fn host_result_helper_reserves_scratch_above_live_caller_registers() {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    for name in [b"type".as_slice(), b"tostring".as_slice()] {
        let (function, high) = {
            let mut parts = runtime.parts_mut().unwrap();
            let (state, gc, strings) = parts.split_mut();
            let key = strings.find_bytes(name).unwrap();
            let function = gc
                .with_ref(state.global_table.unwrap(), |table| {
                    table.get(&Value::String(key))
                })
                .unwrap();
            let high = gc.create(Table::new());
            state.stack.set_top(64);
            state.current_call_info_mut().top = 64;
            *state.stack.at_mut(15).unwrap() = Value::Table(high);
            *state.stack.at_mut(60).unwrap() = Value::Table(high);
            state.top = 1;
            (function, high)
        };
        runtime.parts_mut().unwrap().with_context_gc(|state, gc| {
            lua_vm::execute::call_value_with_results(
                state,
                gc,
                function,
                &[Value::Boolean(true)],
                Some(1),
                |state, gc, values| {
                    assert_eq!(state.top, 1);
                    let [Value::String(value)] = values else {
                        panic!("expected native string result")
                    };
                    let expected = if name == b"type" {
                        b"boolean".as_slice()
                    } else {
                        b"true".as_slice()
                    };
                    assert_eq!(
                        gc.with_string_bytes(*value, |bytes| bytes.to_vec())
                            .unwrap(),
                        expected
                    );
                    assert_eq!(state.stack.at(15), Some(&Value::Table(high)));
                    assert_eq!(state.stack.at(60), Some(&Value::Table(high)));
                },
            )
            .unwrap();
            assert_eq!(state.top, 1);
        });
        runtime.collect_garbage_full().unwrap();
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, _) = parts.split_mut();
        assert!(gc.contains_registered(high));
    }
    assert_eq!(runtime.close().unwrap().remaining_objects, 0);
}

#[test]
fn stack_top_indices_stay_inside_the_frame_and_expansion_initializes_nil() {
    let mut state = lua_vm::LuaState::new();
    state.stack.set_top(16);
    state.current_call_info_mut().base = 4;
    state.current_call_info_mut().top = 16;
    for index in 0..16 {
        *state.stack.at_mut(index).unwrap() = Value::Number(index as f64);
    }
    state.top = 8;
    state.set_top(-1);
    assert_eq!(state.get_top(), 4);
    state.set_top(-2);
    assert_eq!(state.get_top(), 3);
    assert_eq!(state.stack.at(6), Some(&Value::Number(6.0)));
    state.set_top(i32::MIN);
    assert_eq!(state.top, 4);
    assert_eq!(state.get_top(), 0);
    assert_eq!(state.stack.at(3), Some(&Value::Number(3.0)));
    state.set_top(3);
    assert_eq!(state.get_top(), 3);
    for index in 4..7 {
        assert_eq!(state.stack.at(index), Some(&Value::Nil));
    }
    assert_eq!(state.stack.at(15), Some(&Value::Number(15.0)));
    assert_eq!(state.stack.size(), 16);
    state.push_number(42.0);
    assert_eq!(state.get_top(), 4);
    assert_eq!(state.pop(), Some(Value::Number(42.0)));
    state.set_top(-4);
    assert_eq!(state.get_top(), 0);
    assert_eq!(state.pop(), None);
}
