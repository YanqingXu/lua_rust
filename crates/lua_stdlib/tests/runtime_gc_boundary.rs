#![forbid(unsafe_code)]

use lua_compiler::{codegen::CodeGenerator, parser::Parser};
use lua_core::{gc::gc_ref::GcRef, proto::Proto, value::Value};
use lua_vm::Runtime;

fn compile(runtime: &mut Runtime, source: &str) -> GcRef<Proto> {
    let chunk = Parser::new(source).parse().unwrap();
    let mut parts = runtime.parts_mut().unwrap();
    let (_, gc, strings) = parts.split_mut();
    let proto = CodeGenerator::new_with_pool(gc, strings)
        .generate(&chunk, "<runtime-gc-boundary>")
        .unwrap();
    gc.create(proto)
}

#[test]
fn native_calls_collection_and_result_consumption_use_scoped_heap_access() {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    let proto = compile(
        &mut runtime,
        r#"
        local t = {}
        rawset(t, "value", 42)
        local ok, err = pcall(rawset, t, 0/0, 1)
        assert(not ok and type(err) == "string")
        local co = coroutine.create(function()
            local replaced = string.gsub("aa", "a", function() return "b" end)
            coroutine.yield(replaced)
            return 7
        end)
        local resumed, value = coroutine.resume(co)
        assert(resumed and value == "bb")
        resumed, value = coroutine.resume(co)
        assert(resumed and value == 7)
        local order = {3, 1, 2}
        table.sort(order, function(a, b) return a < b end)
        assert(order[1] == 1 and order[3] == 3)
        collectgarbage("collect")
        assert(rawget(t, "value") == 42)
        return string.char(65, 0, 255)
    "#,
    );
    let mut result = Vec::new();
    runtime
        .execute_proto_with_args(proto, Vec::new(), |state, values| {
            let [Value::String(string)] = values else {
                panic!("expected one string")
            };
            result = state.copy_string_bytes(*string).unwrap();
        })
        .unwrap();
    assert_eq!(result, b"A\0\xff");
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
}

#[test]
fn iterative_lua_frames_survive_collection_suspension_and_iterator_return() {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    let proto = compile(
        &mut runtime,
        r#"
        local saved
        local function depth(n)
            local held = {value = n}
            if n == 3 then saved = function() return held.value end end
            if n == 0 then
                collectgarbage("collect")
                return coroutine.yield(10)
            end
            local result = depth(n - 1)
            assert(held.value == n)
            return result + held.value
        end
        local co = coroutine.create(function() return depth(6) end)
        local ok, result = coroutine.resume(co)
        assert(ok and result == 10)
        collectgarbage("collect")
        ok, result = coroutine.resume(co, 9)
        assert(ok and result == 30 and saved() == 3)
        local function step(_, control)
            collectgarbage("collect")
            if control < 2 then return control + 1 end
        end
        local function iterator(state, control) return step(state, control) end
        local total = 0
        for value in iterator, nil, 0 do total = total + value end
        assert(total == 3)
        return result
        "#,
    );
    runtime
        .execute_proto_with_args(proto, Vec::new(), |_, values| {
            assert_eq!(values, &[Value::Number(30.0)]);
        })
        .unwrap();
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
}

#[test]
fn debug_hook_lifecycle_preserves_scoped_state_access() {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    let proto = compile(
        &mut runtime,
        r#"
        local seen, inside, collected = {0, 0}, false, false
        local function leaf(value) return value + 1 end
        local function make_hook(which)
            return function(event)
                assert(event == "line" and not inside)
                inside = true
                seen[which] = seen[which] + 1
                assert(leaf(40) == 41)
                if not collected then collected = true; collectgarbage("collect") end
                inside = false
            end
        end
        local first, second = make_hook(1), make_hook(2)
        local co = coroutine.create(function()
            local self = coroutine.running()
            local held = {value = 40}
            debug.sethook(self, first, "l")
            local installed, mask, count = debug.gethook(self)
            assert(installed == first and mask == "l" and count == 0)
            coroutine.yield("paused")
            local result = leaf(held.value)
            debug.sethook(self)
            assert(debug.gethook(self) == nil)
            return result
        end)
        local ok, value = coroutine.resume(co)
        assert(ok and value == "paused" and seen[1] > 0 and collected)
        local before = seen[1]
        debug.sethook(co, second, "l")
        assert(debug.gethook(co) == second)
        collectgarbage("collect")
        ok, value = coroutine.resume(co)
        assert(ok and value == 41 and seen[1] == before and seen[2] > 0)
        debug.sethook(co)
        assert(debug.gethook(co) == nil and debug.gethook() == nil and not inside)
        return value
        "#,
    );
    runtime
        .execute_proto_with_args(proto, Vec::new(), |_, values| {
            assert_eq!(values, &[Value::Number(41.0)]);
        })
        .unwrap();
    let stats = runtime.activation_stats();
    assert!(stats.peak_callback_helper_depth >= 1);
    assert!(stats.peak_upvalue_transfer_depth >= 1);
    assert_eq!(stats.current_rooted_activation_depth(), 0);
    assert_eq!(runtime.active_execution_count(), 0);
    assert_eq!(runtime.temporary_state_root_count(), 0);
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_coroutine_states, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
}
