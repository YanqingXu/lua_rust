use lua_compiler::codegen::CodeGenerator;
use lua_compiler::parser::Parser;
use lua_core::value::Value;
use lua_stdlib::catalog::open_all;
use lua_vm::Runtime;
use lua_vm::execute::execute_proto;

#[test]
fn load_and_loadstring_keep_lua_provided_chunk_name_bytes() {
    let mut runtime = Runtime::new();
    {
        let mut parts = runtime.parts_mut().expect("runtime parts are available");
        let (state, gc, string_pool) = parts.split_mut();
        open_all(state, gc);

        let source = br#"
            local name = string.char(0, 128, 255)
            local from_string = assert(loadstring("return 1", name))
            local done = false
            local from_reader = assert(load(function()
                if done then return nil end
                done = true
                return "return 2"
            end, name))
            assert(debug.getinfo(from_string).source == name)
            assert(debug.getinfo(from_reader).source == name)
            return debug.getinfo(from_reader).source
        "#;
        let mut parser = Parser::from_bytes(source);
        let chunk = parser.parse().expect("test source should parse");
        let proto = CodeGenerator::new_with_pool(gc, string_pool)
            .generate(&chunk, "<chunk-name-byte-test>")
            .expect("test source should compile");
        let proto = gc.create(proto);
        execute_proto(state, proto, gc).expect("test source should execute");
    }

    let mut parts = runtime.parts_mut().expect("runtime parts remain available");
    let (state, _, _) = parts.split_mut();
    let result = state.stack.at(0).cloned().unwrap_or(Value::Nil);
    let Value::String(result) = result else {
        panic!("expected a string result, got {result:?}");
    };
    assert_eq!(
        state
            .copy_string_bytes(result)
            .expect("result string should be live"),
        [0x00, 0x80, 0xff]
    );
}
