#![forbid(unsafe_code)]

use lua_compiler::{codegen::CodeGenerator, parser::Parser};
use lua_core::value::Value;
use lua_vm::Runtime;

#[test]
fn debug_api_reads_precede_gc_publication() {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    let source = r#"
        local function inspect(value)
            local info = debug.getinfo(1, 'n')
            assert(type(info) == 'table' and type(info.namewhat) == 'string')
            local name, current = debug.getlocal(1, 1)
            assert(name == 'value' and current == 11)
            assert(debug.setlocal(1, 1, 13) == 'value')
            assert(value == 13)
        end
        inspect(11)
        local co = coroutine.create(function()
            local marker = 17
            local self = coroutine.running()
            local info = debug.getinfo(self, 1, 'Sl')
            assert(info.what == 'Lua' and info.currentline > 0)
            local names = debug.getinfo(self, 1, 'n')
            assert(type(names) == 'table' and type(names.namewhat) == 'string')
            local name, value = debug.getlocal(self, 1, 1)
            assert(name == 'marker' and value == 17)
            assert(debug.setlocal(self, 1, 1, 23) == 'marker')
            assert(marker == 23)
            coroutine.yield(marker)
            return marker
        end)
        local ok, value = coroutine.resume(co)
        assert(ok and value == 23)
        local info = debug.getinfo(co, 1, 'Sl')
        assert(info.what == 'Lua' and info.currentline > 0)
        local names = debug.getinfo(co, 1, 'n')
        assert(type(names) == 'table' and type(names.namewhat) == 'string')
        local name, value = debug.getlocal(co, 1, 1)
        assert(name == 'marker' and value == 23)
        assert(debug.setlocal(co, 1, 1, 29) == 'marker')
        collectgarbage('collect')
        ok, value = coroutine.resume(co)
        assert(ok and value == 29)
        return value
    "#;
    let proto = {
        let chunk = Parser::new(source).parse().unwrap();
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, strings) = parts.split_mut();
        let proto = CodeGenerator::new_with_pool(gc, strings)
            .generate(&chunk, "<debug-access-boundary>")
            .unwrap();
        gc.create(proto)
    };
    runtime
        .execute_proto_with_args(proto, Vec::new(), |_, values| {
            assert_eq!(values, &[Value::Number(29.0)]);
        })
        .unwrap();
    assert_eq!(
        runtime.activation_stats().current_rooted_activation_depth(),
        0
    );
    assert_eq!(runtime.active_execution_count(), 0);
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_coroutine_states, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
}
