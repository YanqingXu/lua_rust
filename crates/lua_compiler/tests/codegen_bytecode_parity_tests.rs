use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::proto::Proto;
use lua_core::string_pool::StringPool;

fn with_proto(source: &[u8], check: impl FnOnce(&Proto)) {
    let mut parser = Parser::from_bytes(source);
    let chunk = parser.parse().expect("test source should parse");
    let mut gc = GarbageCollector::new();
    let mut pool = StringPool::new();
    let proto = CodeGenerator::new_with_pool(&mut gc, &mut pool)
        .generate(&chunk, "@codegen-bytecode-parity")
        .expect("test source should compile");

    check(&proto);
}

#[test]
fn call_statement_discards_results_but_expression_call_keeps_one() {
    with_proto(
        b"print(1)\nlocal result = print(2)\nreturn result",
        |proto| {
            let calls = proto
                .code()
                .iter()
                .copied()
                .filter(|inst| opcode::get_opcode(*inst) == OpCode::CALL)
                .collect::<Vec<_>>();

            assert_eq!(calls.len(), 2);
            assert_eq!(opcode::get_arg_b(calls[0]), 2);
            assert_eq!(opcode::get_arg_c(calls[0]), 1);
            assert_eq!(opcode::get_arg_b(calls[1]), 2);
            assert_eq!(opcode::get_arg_c(calls[1]), 2);
        },
    );
}

#[test]
fn representative_basic_chunk_matches_the_cpp_instruction_shape() {
    with_proto(
        br#"
local x = 10
local y = 20
local sum = x + y
local product = x * y
print(sum)
print(product)
return sum
"#,
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 0, 0),
                opcode::create_abx(OpCode::LOADK, 1, 1),
                opcode::create_abc(OpCode::ADD, 2, 0, 1),
                opcode::create_abc(OpCode::MUL, 3, 0, 1),
                opcode::create_abx(OpCode::GETGLOBAL, 4, 2),
                opcode::create_abc(OpCode::MOVE, 5, 2, 0),
                opcode::create_abc(OpCode::CALL, 4, 2, 1),
                opcode::create_abx(OpCode::GETGLOBAL, 4, 2),
                opcode::create_abc(OpCode::MOVE, 5, 3, 0),
                opcode::create_abc(OpCode::CALL, 4, 2, 1),
                opcode::create_abc(OpCode::RETURN, 2, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.constant_count(), 3);
            assert_eq!(proto.max_stack_size(), 6);
        },
    );
}

#[test]
fn contiguous_local_returns_use_the_existing_register_range() {
    with_proto(b"local a,b = 1,2\nreturn a,b", |proto| {
        assert!(
            proto
                .code()
                .iter()
                .all(|inst| opcode::get_opcode(*inst) != OpCode::MOVE)
        );
        assert!(proto.code().iter().copied().any(|inst| {
            opcode::get_opcode(inst) == OpCode::RETURN
                && opcode::get_arg_a(inst) == 0
                && opcode::get_arg_b(inst) == 3
        }));
        assert_eq!(proto.max_stack_size(), 2);
    });
}

#[test]
fn noncontiguous_local_returns_still_materialize_a_result_range() {
    with_proto(b"local a,b,c = 1,2,3\nreturn a,c", |proto| {
        assert!(proto.code().iter().copied().any(|inst| {
            opcode::get_opcode(inst) == OpCode::MOVE
                && opcode::get_arg_a(inst) == 3
                && opcode::get_arg_b(inst) == 0
        }));
        assert!(proto.code().iter().copied().any(|inst| {
            opcode::get_opcode(inst) == OpCode::MOVE
                && opcode::get_arg_a(inst) == 4
                && opcode::get_arg_b(inst) == 2
        }));
        assert!(proto.code().iter().copied().any(|inst| {
            opcode::get_opcode(inst) == OpCode::RETURN
                && opcode::get_arg_a(inst) == 3
                && opcode::get_arg_b(inst) == 3
        }));
    });
}

#[test]
fn max_stack_uses_a_two_slot_minimum_and_the_exact_register_peak() {
    with_proto(b"", |proto| {
        assert_eq!(proto.max_stack_size(), 2);
    });
    with_proto(b"local a = 1\nprint(a)", |proto| {
        assert_eq!(proto.max_stack_size(), 3);
    });
    with_proto(b"local function f() return 1 end\nreturn f", |proto| {
        assert_eq!(proto.sub_proto_count(), 1);
        let child = proto.sub_proto(0);
        // SAFETY: the helper keeps the collector that owns the child Proto
        // alive and does not run collection while this shared borrow exists.
        let child = unsafe { child.as_ref() }.expect("child Proto should be live");
        assert_eq!(child.max_stack_size(), 2);
    });
}
