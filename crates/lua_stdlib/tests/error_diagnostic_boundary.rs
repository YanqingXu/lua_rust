#![forbid(unsafe_code)]

use lua_compiler::{codegen::CodeGenerator, parser::Parser};
use lua_core::{gc::gc_ref::GcRef, proto::Proto, value::Value};
use lua_vm::{Runtime, RuntimeExecutionError};

fn compile(runtime: &mut Runtime, source: &str) -> GcRef<Proto> {
    let chunk = Parser::new(source).parse().unwrap();
    let mut parts = runtime.parts_mut().unwrap();
    let (_, gc, strings) = parts.split_mut();
    let proto = CodeGenerator::new_with_pool(gc, strings)
        .generate(&chunk, "=error-diagnostic-boundary")
        .unwrap();
    gc.create(proto)
}

fn runtime_with_libraries() -> Runtime {
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    runtime
}

#[test]
fn lua_loaders_and_protected_errors_preserve_bytes_and_values() {
    let mut runtime = runtime_with_libraries();
    let proto = compile(
        &mut runtime,
        r#"
        local ff = string.char(255)
        local function diagnostic(value)
            assert(type(value) == 'string')
            assert(string.find(value, ff, 1, true))
            assert(not string.find(value, string.char(239, 191, 189), 1, true))
            assert(not string.find(value, '\\xff', 1, true))
            assert(not string.find(value, '\\xFF', 1, true))
        end
        local fn, syntax_error = loadstring(ff .. 'a = 1', '=raw-source')
        assert(fn == nil)
        diagnostic(syntax_error)
        local pieces, index = {'local x = 1\n', ff, 'a = 1'}, 0
        local reader_error
        fn, reader_error = load(function()
            index = index + 1
            return pieces[index]
        end, '=raw-reader')
        assert(fn == nil)
        diagnostic(reader_error)

        -- A real FF byte also passes through a valid source string literal.
        fn = assert(loadstring("error('" .. ff .. "', 0)", '=raw-value'))
        local ok, value = pcall(fn)
        assert(not ok and value == ff)
        ok, value = pcall(error, ff, 0)
        assert(not ok and value == ff and string.byte(value) == 255)
        ok, value = pcall(function() error(23) end)
        assert(not ok and type(value) == 'string' and string.sub(value, -4) == ': 23')
        ok, value = pcall(error, 23, 0)
        assert(not ok and type(value) == 'number' and value == 23)
        local object = {marker = 17}
        ok, value = pcall(error, object, 0)
        assert(not ok and rawequal(value, object))
        ok, value = pcall(function() error(object) end)
        assert(not ok and rawequal(value, object))
        collectgarbage('collect')
        assert(rawequal(value, object) and value.marker == 17)
        diagnostic(syntax_error)
        diagnostic(reader_error)
        return 1
        "#,
    );
    runtime
        .execute_proto_with_args(proto, Vec::new(), |_, values| {
            assert_eq!(values, &[Value::Number(1.0)]);
        })
        .unwrap();
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
}

#[test]
fn runtime_error_diagnostic_outlives_the_runtime_heap() {
    let mut runtime = runtime_with_libraries();
    let proto = compile(&mut runtime, "error(string.char(255, 0) .. 'owned', 0)");
    let error = runtime
        .execute_proto_with_args(proto, Vec::new(), |_, _| {
            panic!("an unprotected Lua error cannot produce successful results")
        })
        .unwrap_err();
    assert!(matches!(&error, RuntimeExecutionError::Vm(_)));
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
    drop(runtime);

    // Read the message for the first time after its Lua string and collector
    // are gone. Miri catches any lazy dereference of the original error Value.
    assert_eq!(error.message_bytes().as_ref(), b"\xff\0owned");
}
