#![forbid(unsafe_code)]

use lua_compiler::ast::stmt::Chunk;
use lua_compiler::codegen::{CodeGenerator, CodegenError};
use lua_compiler::opcode::OpCode;
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;

fn parse(source: &str) -> Chunk {
    Parser::new(source)
        .parse()
        .expect("these errors must reach code generation")
}

fn compile(source: &str) -> Result<Proto, CodegenError> {
    CodeGenerator::new_with_pool(&mut GarbageCollector::new(), &mut StringPool::new())
        .generate(&parse(source), "=nested-error")
}

fn assert_error(source: &str, expected: &[u8]) {
    let error = compile(source).expect_err("invalid nested code must not produce a Proto");
    assert_eq!(error.message_bytes(), expected, "{source}");
}

macro_rules! rejects_invalid_assignment {
    ($name:ident, $source:literal) => {
        #[test]
        fn $name() {
            assert_error($source, b"Expression is not a valid lvalue");
        }
    };
}

rejects_invalid_assignment!(if_body_rejects_invalid_assignment, "if false then 1=2 end");
rejects_invalid_assignment!(
    elseif_body_rejects_invalid_assignment,
    "if true then local x=1 elseif false then 1=2 end"
);
rejects_invalid_assignment!(
    else_body_rejects_invalid_assignment,
    "if true then local x=1 else 1=2 end"
);
rejects_invalid_assignment!(do_body_rejects_invalid_assignment, "do 1=2 end");
rejects_invalid_assignment!(
    while_body_rejects_invalid_assignment,
    "while false do 1=2 end"
);
rejects_invalid_assignment!(
    repeat_body_rejects_invalid_assignment,
    "repeat 1=2 until true"
);
rejects_invalid_assignment!(
    numeric_for_body_rejects_invalid_assignment,
    "for i=1,0 do 1=2 end"
);
rejects_invalid_assignment!(
    generic_for_body_rejects_invalid_assignment,
    "for key in next,{} do 1=2 end"
);

#[test]
fn errors_escape_multiple_named_and_expression_function_layers() {
    for source in [
        "function outer() do 1=2 end end",
        "local function outer() return function() if false then 1=2 end end end",
        "return function() local function inner() repeat do 1=2 end until true end end",
        "local callbacks={function() while false do 1=2 end end}; return callbacks",
        "consume(function() for key in next,{} do 1=2 end end)",
    ] {
        assert_error(source, b"Expression is not a valid lvalue");
    }
}

#[test]
fn earlier_nested_lvalue_error_precedes_later_break_error() {
    for source in [
        "do 1=2 end; break",
        "if false then 1=2 else break end",
        "repeat 1=2 until true; break",
        "local function bad() do 1=2 end end; break",
        "local bad=function() do 1=2 end end; break",
    ] {
        assert_error(source, b"Expression is not a valid lvalue");
    }
}

#[test]
fn earlier_pending_break_error_precedes_later_nested_lvalue_error() {
    for source in [
        "do break end; if false then 1=2 end",
        "local bad=function() break end; do 1=2 end",
        "local bad=function() do break end end; local later=function() do 1=2 end end",
    ] {
        assert_error(source, b"no loop to break");
    }
}

#[test]
fn same_statement_keeps_an_earlier_pending_error() {
    // Each expression order is checked against the pinned C++ compiler. In the
    // final case, the later invalid target returns a direct lowering error.
    for source in [
        "local a,b = function() break end, function() 1=2 end",
        "return function() break end, function() 1=2 end",
        "consume(function() break end, function() 1=2 end)",
        "local t = {[function() break end] = function() 1=2 end}",
        "a,b = function() break end, function() 1=2 end",
        "t[function() break end], 1 = 2,3",
    ] {
        assert_error(source, b"no loop to break");
    }
}

#[test]
fn same_statement_keeps_an_earlier_direct_error() {
    for source in [
        "local a,b = function() 1=2 end, function() break end",
        "return function() 1=2 end, function() break end",
        "consume(function() 1=2 end, function() break end)",
        "local t = {[function() 1=2 end] = function() break end}",
        "a,b = function() 1=2 end, function() break end",
        "1, t[function() break end] = 2,3",
    ] {
        assert_error(source, b"Expression is not a valid lvalue");
    }
}

#[test]
fn failed_expression_prevents_later_sibling_proto_attachment() {
    for source in [
        "local a,b = function() 1=2 end, function() return 7 end",
        "return function() 1=2 end, function() return 7 end",
        "consume(function() 1=2 end, function() return 7 end)",
        "local t = {[function() 1=2 end] = function() return 7 end}",
        "a,b = function() 1=2 end, function() return 7 end",
        "t[function() 1=2 end] = function() return 7 end",
    ] {
        let mut gc = GarbageCollector::new();
        let mut strings = StringPool::new();
        let mut generator = CodeGenerator::new_with_pool(&mut gc, &mut strings);
        let chunk = parse(source);
        assert_eq!(
            generator.emit_statement(&chunk.statements[0]),
            Err("Expression is not a valid lvalue".to_owned()),
            "{source}"
        );
        assert_eq!(generator.builder.proto().sub_proto_count(), 0, "{source}");
        assert!(generator.parent_functions.is_empty(), "{source}");

        // A fresh child request cannot publish anything after the expression
        // failed, even if the child itself contains entirely valid code.
        let later = parse("return 7");
        assert_eq!(
            generator
                .compile_function(&[], false, &later.statements, 1, 1)
                .expect_err("pending failures must reject later child compilation"),
            "Expression is not a valid lvalue",
            "{source}"
        );
        assert_eq!(generator.builder.proto().sub_proto_count(), 0, "{source}");
    }
}

#[test]
fn failed_function_expressions_in_control_operands_stop_before_the_body() {
    for source in [
        "if (function() do 1=2 end end) then break end",
        "while (function() do 1=2 end end) do local later=function() break end end",
        "for i=(function() do 1=2 end end),1 do local later=function() break end end",
        "for key in (function() do 1=2 end end) do local later=function() break end end",
        "repeat local x=1 until (function() do 1=2 end end)",
    ] {
        assert_error(source, b"Expression is not a valid lvalue");
    }
}

#[test]
fn rejected_child_restores_the_parent_without_attaching_a_partial_proto() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    let mut generator = CodeGenerator::new_with_pool(&mut gc, &mut strings);
    generator.current_line = 17;
    generator.add_local_var("held");
    generator.adjust_local_vars(1);
    generator.add_upvalue("outer", true, 0);
    generator.enter_block(true);
    generator.code_abc(OpCode::LOADBOOL, 0, 1, 0, 17);
    let parent_code = generator.builder.proto().code().to_vec();
    let parent_max = generator.reg_alloc.max_used();
    let body = parse("local bad=function() do 1=2 end end; local later=function() return 3 end");

    let error = generator
        .compile_function(&["argument".to_owned()], true, &body.statements, 4, 9)
        .expect_err("the child must fail before allocation or attachment to its parent");
    assert_eq!(error, "Expression is not a valid lvalue");
    assert_eq!(generator.builder.proto().code(), parent_code);
    assert_eq!(generator.builder.proto().sub_proto_count(), 0);
    assert_eq!(generator.current_line, 17);
    assert_eq!(generator.reg_alloc.current(), 1);
    assert_eq!(generator.reg_alloc.max_used(), parent_max);
    assert_eq!(generator.active_var_count, 1);
    assert_eq!(generator.find_local_var("held"), 0);
    assert_eq!(generator.local_vars.len(), 1);
    assert_eq!(generator.find_upvalue("outer"), 0);
    assert!(generator.parent_functions.is_empty());
    assert_eq!(generator.blocks.len(), 1);
    assert!(generator.blocks[0].is_breakable);
    assert_eq!(generator.blocks[0].active_var_count, 1);
    assert!(generator.blocks[0].breaklist.pcs.is_empty());
    assert_eq!(generator.jpc, -1);
}

#[test]
fn all_valid_nested_control_structures_still_compile() {
    compile(
        r#"
        local held=1
        local function nested(flag)
            if flag then do held=held+1 end
            elseif held<0 then held=0
            else held=held+2 end
            while held<5 do if held==4 then break end; held=held+1 end
            repeat do held=held+1 end until held>5
            for i=1,2 do if i==2 then break end; held=held+i end
            for key,value in next,{x=1} do do held=held+value; break end end
            return function() return held end
        end
        return nested(true)
        "#,
    )
    .expect("only rejected-code paths should change");
}
