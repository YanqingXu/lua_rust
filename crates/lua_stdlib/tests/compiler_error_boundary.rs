#![forbid(unsafe_code)]

use lua_compiler::{codegen::CodeGenerator, parser::Parser};
use lua_core::{gc::collector::GarbageCollector, string_pool::StringPool, value::Value};
use lua_vm::Runtime;

#[test]
fn failed_nested_compilation_releases_the_partial_publication_graph() {
    for (source, expected) in [
        (
            "local prior=function() local x='held'; return function() return x end end; return function() if false then 1=2 end end",
            "Expression is not a valid lvalue",
        ),
        (
            "local prior=function() return 'held' end; return function() local bad=function() do break end end end",
            "no loop to break",
        ),
    ] {
        let chunk = Parser::new(source).parse().unwrap();
        let mut gc = GarbageCollector::new();
        let mut strings = StringPool::new();
        let result = gc.with_publication(|transaction| {
            CodeGenerator::new_in_publication_with_pool(transaction, &mut strings)
                .generate(&chunk, "=rejected-publication")
        });
        let error = result.expect_err("invalid nested code must never publish a top Proto");
        assert_eq!(error.message_bytes(), expected.as_bytes());
        assert!(gc.object_count() > 0, "the failure must follow allocations");
        assert_eq!(gc.temporary_root_count(), 0);
        assert_eq!(gc.rejected_temporary_root_release_count(), 0);
        gc.mark();
        assert_eq!(gc.marked_object_count(), 0);
        assert_eq!(gc.rejected_mark_edge_count(), 0);
        let partial_objects = gc.object_count();
        assert_eq!(gc.sweep(&mut strings), partial_objects);
        assert_eq!(gc.object_count(), 0);
        assert!(strings.is_empty());
        assert_eq!(error.message_bytes(), expected.as_bytes());
    }
}

#[test]
fn loader_rejection_survives_reader_gc_and_later_closure_execution() {
    let source = r#"
        rejected_chunk_effect = 0
        local cases = {
            {"local prior=function() return 41 end; do 1=2 end", "Expression is not a valid lvalue"},
            {"return function() repeat do 1=2 end until true end", "Expression is not a valid lvalue"},
            {"local bad=function() do break end end; do 1=2 end", "no loop to break"}
        }
        for index, case in ipairs(cases) do
            local bad, diagnostic = loadstring("rejected_chunk_effect=1; " .. case[1])
            assert(bad == nil, "invalid source returned a function")
            assert(diagnostic == "loadstring: " .. case[2], diagnostic)
            collectgarbage('collect')
            assert(diagnostic == "loadstring: " .. case[2])
            assert(rejected_chunk_effect == 0)

            local factory = assert(loadstring("local captured=40; return function(delta) captured=captured+delta; return captured end"))
            local closure = factory()
            factory = nil
            collectgarbage('collect')
            assert(closure(index) == 40 + index)
            assert(closure(1) == 41 + index)
        end

        local fragments = {
            "local prior=function() return 41 end; ",
            "return function() repeat ",
            "1=2 until true end"
        }
        local reads = 0
        local bad, diagnostic = load(function()
            reads = reads + 1
            collectgarbage('collect')
            return fragments[reads]
        end, '=reader-with-gc')
        assert(reads == 4)
        assert(bad == nil, "reader published a malformed function")
        assert(diagnostic == 'load: Expression is not a valid lvalue', diagnostic)
        fragments = nil
        collectgarbage('collect')
        assert(diagnostic == 'load: Expression is not a valid lvalue')
        assert(rejected_chunk_effect == 0)

        local factory = assert(loadstring('local held=41; return function() return held+1 end'))
        local closure = factory()
        factory = nil
        collectgarbage('collect')
        return closure()
    "#;
    let chunk = Parser::new(source).parse().unwrap();
    let mut runtime = Runtime::new();
    runtime
        .parts_mut()
        .unwrap()
        .with_context_gc(lua_stdlib::open_all);
    let proto = {
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, strings) = parts.split_mut();
        gc.with_publication(|transaction| {
            let proto = CodeGenerator::new_in_publication_with_pool(transaction, strings)
                .generate(&chunk, "=compiler-error-boundary")
                .unwrap();
            let proto = transaction.alloc(proto);
            transaction.publish_as_explicit_root(proto).unwrap()
        })
    };
    runtime
        .execute_proto_with_args(proto, Vec::new(), |_, values| {
            assert_eq!(values, &[Value::Number(42.0)]);
        })
        .unwrap();
    {
        let mut parts = runtime.parts_mut().unwrap();
        let (_, gc, _) = parts.split_mut();
        assert_eq!(gc.temporary_root_count(), 0);
        assert_eq!(gc.rejected_temporary_root_release_count(), 0);
        gc.remove_root(proto);
    }
    let report = runtime.close().unwrap();
    assert_eq!(report.remaining_objects, 0);
    assert_eq!(report.remaining_allocator_live_bytes, 0);
    assert_eq!(report.remaining_coroutine_states, 0);
    assert_eq!(report.runtime_resource_drain_errors, 0);
}
