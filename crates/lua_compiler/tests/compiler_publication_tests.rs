use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::function::Function;
use lua_core::gc::collector::GarbageCollector;
use lua_core::string_pool::StringPool;

fn parse(source: &[u8]) -> lua_compiler::ast::stmt::Chunk {
    Parser::from_bytes(source)
        .parse()
        .expect("publication fixture should parse")
}

#[test]
fn compiler_proto_tree_publishes_atomically_into_a_lua_function() {
    let chunk = parse(
        br#"
            local outer = "payload"
            return function(argument)
                return outer, argument
            end
        "#,
    );
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();

    let function = gc.with_publication(|transaction| {
        let proto = CodeGenerator::new_in_publication_with_pool(transaction, &mut pool)
            .generate(&chunk, "@compiler-publication")
            .expect("nested fixture should compile");
        let builder_roots = transaction.active_temporary_root_count();
        assert!(
            builder_roots >= 4,
            "source/debug strings and child Proto must be rooted"
        );
        let seed = transaction.trace_mark_only();
        assert_eq!(seed.temporary_seeded, builder_roots);
        assert_eq!(seed.temporary_rejected, 0);

        let proto = transaction.alloc(proto);
        let function = transaction
            .alloc_lua_function(&proto)
            .expect("protected top Proto builds a protected Function");
        assert!(
            transaction
                .function_reaches_proto(&function, &proto)
                .expect("Function→Proto edge validates")
        );
        transaction
            .publish_as_explicit_root(function)
            .expect("Function becomes the traced owner of the Proto tree")
    });

    assert_eq!(gc.temporary_root_count(), 0);
    assert_eq!(gc.rejected_temporary_root_release_count(), 0);
    assert!(gc.is_root(function));
    gc.mark();
    assert_eq!(gc.marked_object_count(), gc.object_count());
    assert_eq!(gc.rejected_mark_edge_count(), 0);
    gc.with_ref(function, |function| {
        assert!(function.proto().is_some());
    })
    .expect("published Function remains registered");

    let object_count = gc.object_count();
    gc.remove_root(function);
    gc.mark();
    assert_eq!(gc.marked_object_count(), 0);
    assert_eq!(gc.sweep(&mut pool), object_count);
    assert_eq!(gc.object_count(), 0);
    assert!(pool.is_empty());
}

#[test]
fn compiler_publication_panic_releases_every_partial_graph_root() {
    let chunk = parse(
        br#"
            local outer = "payload"
            return function(argument)
                return function() return outer, argument end
            end
        "#,
    );
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gc.with_publication(|transaction| {
            let proto = CodeGenerator::new_in_publication_with_pool(transaction, &mut pool)
                .generate(&chunk, "@compiler-publication-panic")
                .expect("nested fixture should compile");
            let proto = transaction.alloc(proto);
            let function = transaction
                .alloc_lua_function(&proto)
                .expect("protected top Proto builds a protected Function");
            assert!(
                transaction
                    .function_reaches_proto(&function, &proto)
                    .expect("Function→Proto edge validates")
            );
            assert!(transaction.active_temporary_root_count() >= 7);
            panic!("injected failure before Function publication");
        });
    }));

    assert!(unwind.is_err());
    assert_eq!(gc.temporary_root_count(), 0);
    assert_eq!(gc.rejected_temporary_root_release_count(), 0);
    gc.mark();
    assert_eq!(gc.marked_object_count(), 0);
    let object_count = gc.object_count();
    assert_eq!(gc.sweep(&mut pool), object_count);
    assert_eq!(gc.object_count(), 0);
    assert!(pool.is_empty());
}

#[test]
fn compiler_consumer_error_drops_builder_roots_without_a_top_proto() {
    let chunk = parse(
        br#"
            local outer = "payload"
            return function()
                return function() return outer end
            end
        "#,
    );
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();

    let result: Result<(), &'static str> = gc.with_publication(|transaction| {
        let _proto = CodeGenerator::new_in_publication_with_pool(transaction, &mut pool)
            .generate(&chunk, "@compiler-publication-error")
            .expect("nested fixture should compile before consumer failure");
        assert!(transaction.active_temporary_root_count() >= 5);
        Err("injected compiler consumer failure")
    });

    assert_eq!(result, Err("injected compiler consumer failure"));
    assert_eq!(gc.temporary_root_count(), 0);
    assert_eq!(gc.rejected_temporary_root_release_count(), 0);
    gc.mark();
    assert_eq!(gc.marked_object_count(), 0);
    let object_count = gc.object_count();
    assert_eq!(gc.sweep(&mut pool), object_count);
    assert_eq!(gc.object_count(), 0);
    assert!(pool.is_empty());
}

#[test]
fn published_function_layout_is_lua_not_native() {
    let chunk = parse(b"return 42");
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();

    let function = gc.with_publication(|transaction| {
        let proto = CodeGenerator::new_in_publication_with_pool(transaction, &mut pool)
            .generate(&chunk, "@compiler-publication-layout")
            .expect("fixture should compile");
        let proto = transaction.alloc(proto);
        let function = transaction
            .alloc_lua_function(&proto)
            .expect("top Proto builds a Lua Function");
        transaction
            .publish_as_explicit_root(function)
            .expect("Function publishes")
    });

    gc.with_ref(function, |function: &Function| {
        assert!(function.is_lua_function());
        assert!(function.proto().is_some());
    })
    .expect("published Function remains registered");
}
