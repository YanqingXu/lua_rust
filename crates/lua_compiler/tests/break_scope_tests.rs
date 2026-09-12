#![forbid(unsafe_code)]

use lua_compiler::codegen::{CodeGenerator, CodegenError};
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;

fn compile(
    gc: &mut GarbageCollector,
    strings: &mut StringPool,
    source: &str,
) -> Result<Proto, CodegenError> {
    let chunk = Parser::new(source)
        .parse()
        .expect("break scope validation belongs to code generation");
    CodeGenerator::new_with_pool(gc, strings).generate(&chunk, "=break-scope")
}

fn assert_no_loop(source: &str) {
    let error = compile(&mut GarbageCollector::new(), &mut StringPool::new(), source)
        .expect_err("a break without a loop must never produce an executable Proto");
    assert_eq!(error.message_bytes(), b"no loop to break", "{source}");
    assert_eq!((error.line, error.column), (0, 0), "{source}");
}

#[test]
fn top_level_and_non_loop_blocks_reject_break() {
    for source in [
        "break",
        "do break end",
        "if true then break end",
        "if false then do break end end",
    ] {
        assert_no_loop(source);
    }
}

#[test]
fn functions_without_a_loop_reject_break_even_when_never_called() {
    for source in [
        "function f() break end",
        "local function f() if false then do break end end end",
        "return function() break end",
    ] {
        assert_no_loop(source);
    }
}

#[test]
fn nested_functions_cannot_break_an_enclosing_functions_loop() {
    for source in [
        "while false do local function f() break end end",
        "repeat local f = function() do break end end until true",
        "for i = 1, 1 do function f() if false then break end end end",
        "for key in next, {} do local function f() break end end",
    ] {
        assert_no_loop(source);
    }
}

#[test]
fn a_later_function_does_not_clear_an_earlier_break_error() {
    assert_no_loop(
        "local bad = function() break end; local good = function() while true do break end end; return good",
    );
    assert_no_loop(
        "local good = function() while true do break end end; local bad = function() break end; return good",
    );
}

#[test]
fn recorded_break_error_precedes_a_later_direct_codegen_error() {
    for source in [
        "local function f() break end; 1 = 2",
        "do break end; 1 = 2",
        "if false then break end; 1 = 2",
    ] {
        assert_no_loop(source);
    }
}

fn assert_patched_jumps(gc: &GarbageCollector, proto: &Proto) {
    for (pc, instruction) in proto.code().iter().copied().enumerate() {
        if opcode::get_opcode(instruction) == OpCode::JMP {
            let offset = opcode::get_arg_sbx(instruction);
            assert_ne!(offset, -1, "these finite-loop fixtures must not self-jump");
            let target = pc as i64 + 1 + i64::from(offset);
            assert!(
                (0..proto.code().len() as i64).contains(&target),
                "jump at {pc} must target an instruction: {target}"
            );
        }
    }
    for index in 0..proto.sub_proto_count() {
        gc.with_ref(proto.sub_proto(index), |child| {
            assert_patched_jumps(gc, child);
        })
        .unwrap();
    }
}

#[test]
fn loops_keep_breaks_inside_nested_if_and_do_blocks() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    for body in [
        "while true do do if true then break end end end",
        "repeat if true then do break end end until false",
        "for i = 1, 3 do do if i == 1 then break end end end",
        "for key in next, {x = 1} do if key then do break end end end",
    ] {
        for source in [
            body.to_owned(),
            format!("local function f() {body} end; f()"),
        ] {
            let proto = compile(&mut gc, &mut strings, &source)
                .expect("a loop in the same function permits break");
            assert_patched_jumps(&gc, &proto);
        }
    }
}

#[test]
fn rejected_generation_does_not_poison_the_next_generator() {
    let mut gc = GarbageCollector::new();
    let mut strings = StringPool::new();
    for _ in 0..2 {
        assert_eq!(
            compile(&mut gc, &mut strings, "return function() break end")
                .unwrap_err()
                .message_bytes(),
            b"no loop to break"
        );
        let proto = compile(&mut gc, &mut strings, "while true do break end; return 1")
            .expect("pending errors belong to one generation, not its allocation services");
        assert_patched_jumps(&gc, &proto);
    }
}
