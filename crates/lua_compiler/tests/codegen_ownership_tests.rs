use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::gc::gc_ref::GcRef;
use lua_core::gc_string::GcString;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;
use lua_core::value::Value;

fn parse(source: &[u8]) -> lua_compiler::ast::stmt::Chunk {
    Parser::from_bytes(source)
        .parse()
        .expect("test source should parse")
}

fn string_constant(proto: &Proto, expected: &[u8]) -> GcRef<GcString> {
    proto
        .constants()
        .iter()
        .find_map(|value| match value {
            Value::String(string) => {
                // SAFETY: every test keeps the owning collector alive while
                // inspecting its generated Proto.
                let bytes = unsafe { string.as_ref() }?.as_bytes();
                (bytes == expected).then_some(*string)
            }
            _ => None,
        })
        .expect("expected string constant should exist")
}

#[test]
fn compiler_reuses_the_explicit_pool_identity() {
    let chunk = parse(br#"return "\000\128\255""#);
    let expected = [0x00, 0x80, 0xff];
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();
    let canonical = pool.intern_bytes(&mut gc, &expected);

    let proto = CodeGenerator::new_with_pool(&mut gc, &mut pool)
        .generate(&chunk, "@pool-identity")
        .expect("code generation should succeed");

    assert_eq!(string_constant(&proto, &expected), canonical);
    assert_eq!(pool.find_bytes(&expected), Some(canonical));
}

#[test]
fn independent_compilers_do_not_cross_heaps_or_pools() {
    let chunk = parse(r#"return "heap-local""#.as_bytes());
    let mut first_gc = GarbageCollector::new();
    let mut first_pool = StringPool::new();
    let mut second_gc = GarbageCollector::new();
    let mut second_pool = StringPool::new();

    let first_proto = CodeGenerator::new_with_pool(&mut first_gc, &mut first_pool)
        .generate(&chunk, "@first")
        .expect("first code generation should succeed");
    let second_proto = CodeGenerator::new_with_pool(&mut second_gc, &mut second_pool)
        .generate(&chunk, "@second")
        .expect("second code generation should succeed");

    let first = string_constant(&first_proto, b"heap-local");
    let second = string_constant(&second_proto, b"heap-local");
    assert_ne!(first, second);
    assert_eq!(first_pool.find_bytes(b"heap-local"), Some(first));
    assert_eq!(second_pool.find_bytes(b"heap-local"), Some(second));
}

#[test]
fn proto_source_preserves_arbitrary_lua_bytes() {
    let chunk = parse(b"return 1");
    let source_name = [0x00, 0x80, 0xff];
    let mut gc = GarbageCollector::new();

    let proto = CodeGenerator::new(&mut gc)
        .generate_with_source_bytes(&chunk, &source_name)
        .expect("code generation should succeed");
    let source = proto.source().expect("Proto should retain its source name");
    // SAFETY: the collector remains live for this assertion.
    let source = unsafe { source.as_ref() }.expect("source string should be live");

    assert_eq!(source.as_bytes(), source_name);
}
