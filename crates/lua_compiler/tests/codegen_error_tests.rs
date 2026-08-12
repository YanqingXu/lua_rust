use lua_compiler::ast::expr::{CallExpr, Expr, NameExpr, SourceLocation};
use lua_compiler::ast::stmt::{CallStmt, Chunk, Stmt};
use lua_compiler::codegen::{CodeGenerator, CodegenError};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::string_pool::StringPool;

fn generate_chunk_error(chunk: &Chunk) -> CodegenError {
    let mut gc = GarbageCollector::new();
    let mut string_pool = StringPool::new();
    CodeGenerator::new_with_pool(&mut gc, &mut string_pool)
        .generate(chunk, "@codegen-error.lua")
        .expect_err("fixture should fail code generation")
}

fn generate_error(source: &[u8]) -> CodegenError {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser
        .parse()
        .expect("fixture should reach code generation");
    generate_chunk_error(&chunk)
}

#[test]
fn invalid_lvalue_is_a_structured_codegen_error() {
    let error = generate_error(b"1 = 2");

    assert_eq!(error.message, "Expression is not a valid lvalue");
    assert_eq!(error.line, 0);
    assert_eq!(error.column, 0);
}

#[test]
fn invalid_lvalue_inside_function_expression_restores_parent_and_returns_error() {
    let error = generate_error(b"return function() 1 = 2 end");

    assert_eq!(error.message, "Expression is not a valid lvalue");
    assert_eq!(error.line, 0);
    assert_eq!(error.column, 0);
}

#[test]
fn malformed_method_call_ast_is_a_structured_codegen_error() {
    let location = SourceLocation::new(7, 3);
    let call = Expr::Call(CallExpr {
        location,
        func: Box::new(Expr::Name(NameExpr {
            location,
            name: "not_a_member".to_string(),
        })),
        args: Vec::new(),
        is_method_call: true,
    });
    let chunk = Chunk {
        statements: vec![Box::new(Stmt::Call(CallStmt {
            location,
            call: Box::new(call),
        }))],
    };

    let error = generate_chunk_error(&chunk);
    assert_eq!(error.message, "Method call must have MemberExpr as func");
    assert_eq!(error.line, 0);
    assert_eq!(error.column, 0);
}

#[test]
fn long_control_structure_returns_codegen_error_instead_of_panicking() {
    let mut source = String::with_capacity(280_020);
    source.push_str("if false then\n");
    for _ in 0..70_000 {
        source.push_str("a=1\n");
    }
    source.push_str("end\n");

    let mut parser = Parser::from_bytes(source.as_bytes());
    let chunk = parser.parse().expect("large fixture remains valid Lua");
    let mut gc = GarbageCollector::new();
    let mut string_pool = StringPool::new();

    let error = CodeGenerator::new_with_pool(&mut gc, &mut string_pool)
        .generate(&chunk, "@control-structure-too-long.lua")
        .expect_err("out-of-range jump must reject the partial Proto");

    assert_eq!(error.message, "control structure too long");
    assert_eq!(error.line, 0);
    assert_eq!(error.column, 0);
}
