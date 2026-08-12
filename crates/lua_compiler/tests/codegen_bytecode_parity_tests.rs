use lua_compiler::codegen::CodeGenerator;
use lua_compiler::opcode::{self, OpCode};
use lua_compiler::parser::Parser;
use lua_core::gc::collector::GarbageCollector;
use lua_core::proto::{Proto, VARARG_HASARG, VARARG_ISVARARG, VARARG_NEEDSARG};
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

#[test]
fn open_table_fields_do_not_inflate_newtable_array_hint() {
    for source in [
        &b"local function f(...) return { n = 0, ... } end"[..],
        &b"local function f() return { n = 0, g() } end"[..],
    ] {
        with_proto(source, |proto| {
            assert_eq!(proto.sub_proto_count(), 1);
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the collector that owns the child Proto
            // alive and does not run collection while this shared borrow exists.
            let child = unsafe { child.as_ref() }.expect("child Proto should be live");
            let newtable = child
                .code()
                .iter()
                .copied()
                .find(|inst| opcode::get_opcode(*inst) == OpCode::NEWTABLE)
                .expect("table constructor should emit NEWTABLE");

            assert_eq!(opcode::get_arg_b(newtable), 0);
            assert_eq!(opcode::get_arg_c(newtable), 1);
        });
    }
}

#[test]
fn indexed_assignment_matches_cpp_lvalue_freezing_order() {
    with_proto(
        b"local history\nlocal function remember(value)\n\
          history.n = history.n + 1\n\
          history[history.n] = value\n\
          end",
        |proto| {
            assert_eq!(proto.sub_proto_count(), 1);
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the collector that owns the child Proto
            // alive and does not run collection while this shared borrow exists.
            let child = unsafe { child.as_ref() }.expect("child Proto should be live");
            let expected = [
                opcode::create_abc(OpCode::GETUPVAL, 1, 0, 0),
                opcode::create_abc(OpCode::MOVE, 2, 1, 0),
                opcode::create_abc(OpCode::GETUPVAL, 3, 0, 0),
                opcode::create_abc(OpCode::GETTABLE, 4, 3, opcode::rk_ask(0)),
                opcode::create_abc(OpCode::ADD, 4, 4, opcode::rk_ask(1)),
                opcode::create_abc(OpCode::SETTABLE, 2, opcode::rk_ask(0), 4),
                opcode::create_abc(OpCode::GETUPVAL, 1, 0, 0),
                opcode::create_abc(OpCode::GETUPVAL, 2, 0, 0),
                opcode::create_abc(OpCode::GETTABLE, 3, 2, opcode::rk_ask(0)),
                opcode::create_abc(OpCode::MOVE, 4, 1, 0),
                opcode::create_abc(OpCode::MOVE, 5, 3, 0),
                opcode::create_abc(OpCode::SETTABLE, 4, 5, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(child.code(), expected);
        },
    );
}

#[test]
fn upvalue_arithmetic_assignment_uses_cpp_relocatable_result_register() {
    with_proto(
        b"local counter\nlocal function increment() counter = counter + 1 end",
        |proto| {
            assert_eq!(proto.sub_proto_count(), 1);
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the collector that owns the child Proto
            // alive and does not run collection while this shared borrow exists.
            let child = unsafe { child.as_ref() }.expect("child Proto should be live");
            let expected = [
                opcode::create_abc(OpCode::GETUPVAL, 0, 0, 0),
                opcode::create_abc(OpCode::ADD, 1, 0, opcode::rk_ask(0)),
                opcode::create_abc(OpCode::SETUPVAL, 1, 0, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(child.code(), expected);
        },
    );
}

#[test]
fn for_loops_emit_cpp_iteration_scope_close_contract() {
    with_proto(
        br#"
local function numeric_empty(n)
    for i = 1, n do end
end
local function numeric_nonempty(n)
    for i = 1, n do n = n + i end
end
local function generic_empty(iter, state, control)
    for key in iter, state, control do end
end
local function generic_nonempty(iter, state, control)
    for key in iter, state, control do control = key end
end
"#,
        |proto| {
            assert_eq!(proto.sub_proto_count(), 4);
            let expected = [
                vec![
                    OpCode::LOADK,
                    OpCode::MOVE,
                    OpCode::LOADK,
                    OpCode::FORPREP,
                    OpCode::FORLOOP,
                    OpCode::RETURN,
                ],
                vec![
                    OpCode::LOADK,
                    OpCode::MOVE,
                    OpCode::LOADK,
                    OpCode::FORPREP,
                    OpCode::ADD,
                    OpCode::CLOSE,
                    OpCode::FORLOOP,
                    OpCode::RETURN,
                ],
                vec![
                    OpCode::MOVE,
                    OpCode::MOVE,
                    OpCode::MOVE,
                    OpCode::JMP,
                    OpCode::CLOSE,
                    OpCode::TFORLOOP,
                    OpCode::JMP,
                    OpCode::RETURN,
                ],
                vec![
                    OpCode::MOVE,
                    OpCode::MOVE,
                    OpCode::MOVE,
                    OpCode::JMP,
                    OpCode::MOVE,
                    OpCode::CLOSE,
                    OpCode::TFORLOOP,
                    OpCode::JMP,
                    OpCode::RETURN,
                ],
            ];

            for (index, expected_opcodes) in expected.iter().enumerate() {
                let child = proto.sub_proto(index);
                // SAFETY: the helper keeps the collector that owns the child Proto
                // alive and does not run collection while this shared borrow exists.
                let child = unsafe { child.as_ref() }.expect("child Proto should be live");
                let actual = child
                    .code()
                    .iter()
                    .copied()
                    .map(opcode::get_opcode)
                    .collect::<Vec<_>>();
                assert_eq!(&actual, expected_opcodes, "child proto {index}");
            }

            let numeric_close = proto.sub_proto(1);
            // SAFETY: same collector lifetime as above.
            let numeric_close = unsafe { numeric_close.as_ref() }.unwrap().code()[5];
            assert_eq!(opcode::get_arg_a(numeric_close), 4);

            let generic_close = proto.sub_proto(2);
            // SAFETY: same collector lifetime as above.
            let generic_code = unsafe { generic_close.as_ref() }.unwrap().code();
            assert_eq!(opcode::get_arg_sbx(generic_code[3]), 1);
            assert_eq!(opcode::get_arg_a(generic_code[4]), 6);

            let generic_nonempty = proto.sub_proto(3);
            // SAFETY: same collector lifetime as above.
            let generic_nonempty = unsafe { generic_nonempty.as_ref() }.unwrap().code();
            assert_eq!(opcode::get_arg_sbx(generic_nonempty[3]), 2);
        },
    );
}

#[test]
fn boolean_comparison_chain_uses_cpp_short_circuit_jump_lists() {
    with_proto(
        b"local a, b, c = ...\nreturn a == 1 and b == 2 and c == 3",
        |proto| {
            let expected = [
                opcode::create_abc(OpCode::VARARG, 0, 4, 0),
                opcode::create_abc(OpCode::EQ, 0, 0, opcode::rk_ask(0)),
                opcode::create_as_bx(OpCode::JMP, 0, 4),
                opcode::create_abc(OpCode::EQ, 0, 1, opcode::rk_ask(1)),
                opcode::create_as_bx(OpCode::JMP, 0, 2),
                opcode::create_abc(OpCode::EQ, 1, 2, opcode::rk_ask(2)),
                opcode::create_as_bx(OpCode::JMP, 0, 1),
                opcode::create_abc(OpCode::LOADBOOL, 3, 0, 1),
                opcode::create_abc(OpCode::LOADBOOL, 3, 1, 0),
                opcode::create_abc(OpCode::RETURN, 3, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
        },
    );
}

#[test]
fn root_scope_upvalues_close_before_the_implicit_return() {
    with_proto(
        b"local captured = 1\nlocal function read()\nreturn captured\nend",
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 0, 0),
                opcode::create_abx(OpCode::CLOSURE, 1, 0),
                opcode::create_abc(OpCode::MOVE, 0, 0, 0),
                opcode::create_abc(OpCode::CLOSE, 0, 0, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[1, 2, 2, 0, 4]);

            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let child = unsafe { child.as_ref() }.unwrap();
            assert_eq!(child.line_info().last(), Some(&4));
        },
    );
}

#[test]
fn while_back_edge_uses_the_cpp_synthetic_line_zero() {
    with_proto(
        b"local flag = true\nwhile flag do flag = false end",
        |proto| {
            let (pc, _) = proto
                .code()
                .iter()
                .copied()
                .enumerate()
                .find(|(_, inst)| {
                    opcode::get_opcode(*inst) == OpCode::JMP && opcode::get_arg_sbx(*inst) < 0
                })
                .expect("while loop should emit a backward JMP");
            assert_eq!(proto.line_info()[pc], 0);
        },
    );
}

#[test]
fn table_array_fields_use_cpp_target_register_scheduling() {
    with_proto(b"return { x, x + 10, x + 20 }", |proto| {
        let expected = [
            opcode::create_abc(OpCode::NEWTABLE, 0, 3, 0),
            opcode::create_abx(OpCode::GETGLOBAL, 1, 0),
            opcode::create_abx(OpCode::GETGLOBAL, 2, 0),
            opcode::create_abc(OpCode::ADD, 2, 2, opcode::rk_ask(1)),
            opcode::create_abx(OpCode::GETGLOBAL, 3, 0),
            opcode::create_abc(OpCode::ADD, 3, 3, opcode::rk_ask(2)),
            opcode::create_abc(OpCode::SETLIST, 0, 3, 1),
            opcode::create_abc(OpCode::RETURN, 0, 2, 0),
            opcode::create_abc(OpCode::RETURN, 0, 1, 0),
        ];
        assert_eq!(proto.code(), expected);
    });

    with_proto(b"return {\n x,\n x + 10,\n x + 20\n}", |proto| {
        assert_eq!(proto.line_info(), &[1, 1, 3, 3, 4, 4, 1, 1, 1])
    });
}

#[test]
fn multi_operand_concat_uses_one_cpp_register_range() {
    with_proto(
        b"local function join(left, right) return left .. ':' .. right end",
        |proto| {
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let child = unsafe { child.as_ref() }.unwrap();
            let expected = [
                opcode::create_abc(OpCode::MOVE, 2, 0, 0),
                opcode::create_abx(OpCode::LOADK, 3, 0),
                opcode::create_abc(OpCode::MOVE, 4, 1, 0),
                opcode::create_abc(OpCode::CONCAT, 2, 2, 4),
                opcode::create_abc(OpCode::RETURN, 2, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];
            assert_eq!(child.code(), expected);
        },
    );
}

#[test]
fn explicit_call_base_moves_member_callee_from_cpp_temporary_register() {
    with_proto(
        b"local function make()\n    return object.make(function() end)\nend",
        |proto| {
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let child = unsafe { child.as_ref() }.unwrap();
            let expected = [
                opcode::create_abx(OpCode::GETGLOBAL, 0, 0),
                opcode::create_abc(OpCode::GETTABLE, 1, 0, opcode::rk_ask(1)),
                opcode::create_abc(OpCode::MOVE, 0, 1, 0),
                opcode::create_abx(OpCode::CLOSURE, 1, 0),
                opcode::create_abc(OpCode::TAILCALL, 0, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 0, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(child.code(), expected);
            assert_eq!(child.line_info(), &[2, 2, 2, 2, 2, 2, 3]);
        },
    );
}

#[test]
fn local_call_results_move_from_the_cpp_temporary_call_frame() {
    with_proto(b"local result = object.make(function() end)", |proto| {
        let expected = [
            opcode::create_abx(OpCode::GETGLOBAL, 0, 0),
            opcode::create_abc(OpCode::GETTABLE, 1, 0, opcode::rk_ask(1)),
            opcode::create_abx(OpCode::CLOSURE, 2, 0),
            opcode::create_abc(OpCode::CALL, 1, 2, 2),
            opcode::create_abc(OpCode::MOVE, 0, 1, 0),
            opcode::create_abc(OpCode::RETURN, 0, 1, 0),
        ];

        assert_eq!(proto.code(), expected);
        assert_eq!(proto.line_info(), &[1; 6]);
        assert_eq!(proto.max_stack_size(), 3);
    });
}

#[test]
fn nil_comparisons_use_the_cpp_rk_constant_without_loadnil() {
    with_proto(b"local value = ...\nreturn value ~= nil", |proto| {
        let expected = [
            opcode::create_abc(OpCode::VARARG, 0, 2, 0),
            opcode::create_abc(OpCode::EQ, 0, 0, opcode::rk_ask(0)),
            opcode::create_as_bx(OpCode::JMP, 0, 1),
            opcode::create_abc(OpCode::LOADBOOL, 1, 0, 1),
            opcode::create_abc(OpCode::LOADBOOL, 1, 1, 0),
            opcode::create_abc(OpCode::RETURN, 1, 2, 0),
            opcode::create_abc(OpCode::RETURN, 0, 1, 0),
        ];

        assert_eq!(proto.code(), expected);
        assert_eq!(proto.constant_count(), 1);
        assert!(
            proto
                .code()
                .iter()
                .all(|inst| opcode::get_opcode(*inst) != OpCode::LOADNIL)
        );
    });
}

#[test]
fn nested_call_materialization_keeps_the_inner_cpp_source_line() {
    with_proto(
        b"local function invoke(node)\n    return outer(\n        inner(node)\n    )\nend",
        |proto| {
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let child = unsafe { child.as_ref() }.unwrap();
            let expected = [
                opcode::create_abx(OpCode::GETGLOBAL, 1, 0),
                opcode::create_abx(OpCode::GETGLOBAL, 2, 1),
                opcode::create_abc(OpCode::MOVE, 3, 0, 0),
                opcode::create_abc(OpCode::CALL, 2, 2, 0),
                opcode::create_abc(OpCode::TAILCALL, 1, 0, 0),
                opcode::create_abc(OpCode::RETURN, 1, 0, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(child.code(), expected);
            assert_eq!(child.line_info(), &[2, 3, 3, 3, 2, 2, 5]);
        },
    );
}

#[test]
fn parenthesized_conditions_stay_in_the_cpp_jump_list_pipeline() {
    with_proto(
        b"local p = true\nlocal q = false\nlocal hit = false\nif (p or q) and (not q) then\n    hit = true\nend\nreturn hit",
        |proto| {
            let expected = [
                opcode::create_abc(OpCode::LOADBOOL, 0, 1, 0),
                opcode::create_abc(OpCode::LOADBOOL, 1, 0, 0),
                opcode::create_abc(OpCode::LOADBOOL, 2, 0, 0),
                opcode::create_abc(OpCode::TEST, 0, 0, 1),
                opcode::create_as_bx(OpCode::JMP, 0, 2),
                opcode::create_abc(OpCode::TEST, 1, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, 3),
                opcode::create_abc(OpCode::TEST, 1, 0, 1),
                opcode::create_as_bx(OpCode::JMP, 0, 1),
                opcode::create_abc(OpCode::LOADBOOL, 2, 1, 0),
                opcode::create_abc(OpCode::RETURN, 2, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[1, 2, 3, 4, 4, 4, 4, 4, 4, 5, 7, 7]);
            assert_eq!(proto.max_stack_size(), 4);
        },
    );
}

#[test]
fn if_branch_exit_jump_uses_the_cpp_end_line() {
    with_proto(
        b"local branch = 0\nif false then\n    branch = -100\nelse\n    branch = branch + 1\nend\nreturn branch",
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, 2),
                opcode::create_abx(OpCode::LOADK, 0, 1),
                opcode::create_as_bx(OpCode::JMP, 0, 1),
                opcode::create_abc(OpCode::ADD, 0, 0, opcode::rk_ask(2)),
                opcode::create_abc(OpCode::RETURN, 0, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[1, 2, 3, 6, 5, 7, 7]);
        },
    );
}

#[test]
fn repeat_condition_uses_the_cpp_value_materialization_pipeline() {
    with_proto(
        b"local repeatCount = 0\nrepeat\n    repeatCount = repeatCount + 1\nuntil repeatCount == 2 or mark(false)\nreturn repeatCount",
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 0, 0),
                opcode::create_abc(OpCode::ADD, 0, 0, opcode::rk_ask(1)),
                opcode::create_abc(OpCode::EQ, 1, 0, opcode::rk_ask(2)),
                opcode::create_as_bx(OpCode::JMP, 0, 1),
                opcode::create_abc(OpCode::LOADBOOL, 1, 0, 1),
                opcode::create_abc(OpCode::LOADBOOL, 1, 1, 0),
                opcode::create_abc(OpCode::TEST, 1, 0, 1),
                opcode::create_as_bx(OpCode::JMP, 0, 4),
                opcode::create_abx(OpCode::GETGLOBAL, 2, 3),
                opcode::create_abc(OpCode::LOADBOOL, 3, 0, 0),
                opcode::create_abc(OpCode::CALL, 2, 2, 2),
                opcode::create_abc(OpCode::MOVE, 1, 2, 0),
                opcode::create_abc(OpCode::TEST, 1, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, -13),
                opcode::create_abc(OpCode::RETURN, 0, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[1, 3, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5]);
            assert_eq!(proto.max_stack_size(), 4);
        },
    );
}

#[test]
fn repeat_literal_conditions_keep_the_cpp_special_cases() {
    with_proto(
        b"local function until_true()\n    repeat\n        local value\n    until true\nend\n\nlocal function until_false()\n    repeat\n        local value\n    until false\nend\n\nlocal function until_nil()\n    repeat\n        local value\n    until nil\nend",
        |proto| {
            assert_eq!(proto.sub_proto_count(), 3);

            let until_true = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let until_true = unsafe { until_true.as_ref() }.unwrap();
            assert_eq!(
                until_true.code(),
                [
                    opcode::create_abc(OpCode::LOADNIL, 0, 0, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 1, 0),
                ]
            );
            assert_eq!(until_true.line_info(), &[0, 5]);

            for (index, repeat_line, condition_line, return_line) in
                [(1, 8, 10, 11), (2, 14, 16, 17)]
            {
                let child = proto.sub_proto(index);
                // SAFETY: the helper keeps the owning collector alive.
                let child = unsafe { child.as_ref() }.unwrap();
                assert_eq!(
                    child.code(),
                    [
                        opcode::create_abc(OpCode::LOADNIL, 0, 0, 0),
                        opcode::create_abc(OpCode::LOADBOOL, 1, 0, 0),
                        opcode::create_abc(OpCode::TEST, 1, 0, 0),
                        opcode::create_as_bx(OpCode::JMP, 0, -4),
                        opcode::create_abc(OpCode::RETURN, 0, 1, 0),
                    ]
                );
                assert_eq!(
                    child.line_info(),
                    &[0, repeat_line, condition_line, condition_line, return_line]
                );
            }
        },
    );
}

#[test]
fn return_values_reset_cpp_temporaries_after_condition_lowering() {
    with_proto(
        b"local function choose(value)\n    if value == holder.answer then\n        return \"matched\"\n    end\n    return \"other\"\nend",
        |proto| {
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let child = unsafe { child.as_ref() }.unwrap();
            let expected = [
                opcode::create_abx(OpCode::GETGLOBAL, 1, 0),
                opcode::create_abc(OpCode::GETTABLE, 2, 1, opcode::rk_ask(1)),
                opcode::create_abc(OpCode::EQ, 0, 0, 2),
                opcode::create_as_bx(OpCode::JMP, 0, 2),
                opcode::create_abx(OpCode::LOADK, 1, 2),
                opcode::create_abc(OpCode::RETURN, 1, 2, 0),
                opcode::create_abx(OpCode::LOADK, 1, 3),
                opcode::create_abc(OpCode::RETURN, 1, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(child.code(), expected);
            assert_eq!(child.line_info(), &[2, 2, 2, 2, 3, 3, 5, 5, 6]);
            assert_eq!(child.max_stack_size(), 3);
        },
    );
}

#[test]
fn local_nil_initialization_matches_cpp_liveness_contract() {
    with_proto(
        b"local function entry(flag)\n    local value\n    if flag then\n        value = 1\n    else\n        value = 2\n    end\n    return value\nend\n\nlocal function dead()\n    side_effect()\n    local unused\n    return 1\nend\n\nlocal function read()\n    side_effect()\n    local value\n    return value\nend\n\nreturn entry, dead, read",
        |proto| {
            assert_eq!(proto.sub_proto_count(), 3);

            let entry = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let entry = unsafe { entry.as_ref() }.unwrap();
            assert_eq!(
                entry.code(),
                [
                    opcode::create_abc(OpCode::TEST, 0, 0, 0),
                    opcode::create_as_bx(OpCode::JMP, 0, 2),
                    opcode::create_abx(OpCode::LOADK, 1, 0),
                    opcode::create_as_bx(OpCode::JMP, 0, 1),
                    opcode::create_abx(OpCode::LOADK, 1, 1),
                    opcode::create_abc(OpCode::RETURN, 1, 2, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 1, 0),
                ]
            );
            assert_eq!(entry.line_info(), &[3, 3, 4, 7, 6, 8, 9]);

            let dead = proto.sub_proto(1);
            // SAFETY: the helper keeps the owning collector alive.
            let dead = unsafe { dead.as_ref() }.unwrap();
            assert_eq!(
                dead.code(),
                [
                    opcode::create_abx(OpCode::GETGLOBAL, 0, 0),
                    opcode::create_abc(OpCode::CALL, 0, 1, 1),
                    opcode::create_abx(OpCode::LOADK, 1, 1),
                    opcode::create_abc(OpCode::RETURN, 1, 2, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 1, 0),
                ]
            );
            assert_eq!(dead.line_info(), &[12, 12, 14, 14, 15]);

            let read = proto.sub_proto(2);
            // SAFETY: the helper keeps the owning collector alive.
            let read = unsafe { read.as_ref() }.unwrap();
            assert_eq!(
                read.code(),
                [
                    opcode::create_abx(OpCode::GETGLOBAL, 0, 0),
                    opcode::create_abc(OpCode::CALL, 0, 1, 1),
                    opcode::create_abc(OpCode::LOADNIL, 0, 0, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 2, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 1, 0),
                ]
            );
            assert_eq!(read.line_info(), &[18, 18, 0, 20, 21]);
        },
    );
}

#[test]
fn legacy_vararg_arg_slot_uses_all_cpp_flags() {
    with_proto(
        b"local function legacy(...) return 1 end\nlocal function modern(...) return ... end",
        |proto| {
            assert_eq!(proto.sub_proto_count(), 2);

            let legacy = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let legacy = unsafe { legacy.as_ref() }.unwrap();
            assert_eq!(
                legacy.vararg_flags(),
                VARARG_HASARG | VARARG_ISVARARG | VARARG_NEEDSARG
            );

            let modern = proto.sub_proto(1);
            // SAFETY: the helper keeps the owning collector alive.
            let modern = unsafe { modern.as_ref() }.unwrap();
            assert_eq!(modern.vararg_flags(), VARARG_ISVARARG);
        },
    );
}

#[test]
fn do_scope_close_uses_the_cpp_opening_line() {
    with_proto(
        b"do\n    local captured = 1\n    local function closure()\n        return captured\n    end\n    consume(closure)\nend",
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 0, 0),
                opcode::create_abx(OpCode::CLOSURE, 1, 0),
                opcode::create_abc(OpCode::MOVE, 0, 0, 0),
                opcode::create_abx(OpCode::GETGLOBAL, 2, 1),
                opcode::create_abc(OpCode::MOVE, 3, 1, 0),
                opcode::create_abc(OpCode::CALL, 2, 2, 1),
                opcode::create_abc(OpCode::CLOSE, 0, 0, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[2, 3, 3, 6, 6, 6, 1, 7]);
        },
    );
}

#[test]
fn while_break_closes_captured_locals_on_both_exits() {
    with_proto(
        b"local closure\nwhile true do\n    local captured = 7\n    closure = function()\n        return captured\n    end\n    break\nend\nreturn closure",
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 1, 0),
                opcode::create_abx(OpCode::CLOSURE, 2, 0),
                opcode::create_abc(OpCode::MOVE, 0, 1, 0),
                opcode::create_abc(OpCode::MOVE, 0, 2, 0),
                opcode::create_abc(OpCode::CLOSE, 1, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, 2),
                opcode::create_abc(OpCode::CLOSE, 1, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, -8),
                opcode::create_abc(OpCode::RETURN, 0, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[3, 4, 4, 4, 7, 7, 2, 0, 9, 9]);
        },
    );
}

#[test]
fn nonlocal_function_statement_stores_through_a_cpp_temporary() {
    with_proto(
        b"local closure\nfunction closure()\n    return 1\nend\nreturn closure",
        |proto| {
            assert_eq!(
                proto.code(),
                [
                    opcode::create_abx(OpCode::CLOSURE, 1, 0),
                    opcode::create_abc(OpCode::MOVE, 0, 1, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 2, 0),
                    opcode::create_abc(OpCode::RETURN, 0, 1, 0),
                ]
            );
            assert_eq!(proto.line_info(), &[2, 2, 5, 5]);
            assert_eq!(proto.max_stack_size(), 2);
        },
    );
}

#[test]
fn for_break_skips_the_loop_scope_close_already_done_by_break() {
    with_proto(
        b"local closure\nfor index = 1, 3 do\n    closure = function()\n        return index\n    end\n    break\nend\nreturn closure",
        |proto| {
            let expected = [
                opcode::create_abx(OpCode::LOADK, 1, 0),
                opcode::create_abx(OpCode::LOADK, 2, 1),
                opcode::create_abx(OpCode::LOADK, 3, 0),
                opcode::create_as_bx(OpCode::FORPREP, 1, 6),
                opcode::create_abx(OpCode::CLOSURE, 5, 0),
                opcode::create_abc(OpCode::MOVE, 0, 4, 0),
                opcode::create_abc(OpCode::MOVE, 0, 5, 0),
                opcode::create_abc(OpCode::CLOSE, 1, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, 3),
                opcode::create_abc(OpCode::CLOSE, 4, 0, 0),
                opcode::create_as_bx(OpCode::FORLOOP, 1, -7),
                opcode::create_abc(OpCode::CLOSE, 1, 0, 0),
                opcode::create_abc(OpCode::RETURN, 0, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(
                proto.line_info(),
                &[2, 2, 2, 2, 3, 3, 3, 6, 6, 2, 2, 2, 8, 8]
            );
        },
    );
}

#[test]
fn if_scope_close_uses_the_cpp_statement_line() {
    with_proto(
        b"local function choose(flag)\n    local closure\n    if flag then\n        local captured = 1\n        closure = function()\n            return captured\n        end\n    end\n    return closure\nend",
        |proto| {
            let child = proto.sub_proto(0);
            // SAFETY: the helper keeps the owning collector alive.
            let child = unsafe { child.as_ref() }.unwrap();
            let expected = [
                opcode::create_abc(OpCode::TEST, 0, 0, 0),
                opcode::create_as_bx(OpCode::JMP, 0, 5),
                opcode::create_abx(OpCode::LOADK, 2, 0),
                opcode::create_abx(OpCode::CLOSURE, 3, 0),
                opcode::create_abc(OpCode::MOVE, 0, 2, 0),
                opcode::create_abc(OpCode::MOVE, 1, 3, 0),
                opcode::create_abc(OpCode::CLOSE, 2, 0, 0),
                opcode::create_abc(OpCode::RETURN, 1, 2, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(child.code(), expected);
            assert_eq!(child.line_info(), &[3, 3, 4, 5, 5, 5, 3, 9, 10]);
        },
    );
}

#[test]
fn parenthesized_immediates_stay_foldable_like_cpp() {
    with_proto(b"return 2^(3^2), (2^3)*4, (-2)^2", |proto| {
        let expected = [
            opcode::create_abx(OpCode::LOADK, 0, 0),
            opcode::create_abx(OpCode::LOADK, 1, 1),
            opcode::create_abx(OpCode::LOADK, 2, 2),
            opcode::create_abc(OpCode::RETURN, 0, 4, 0),
            opcode::create_abc(OpCode::RETURN, 0, 1, 0),
        ];

        assert_eq!(proto.code(), expected);
        assert_eq!(proto.line_info(), &[1; 5]);
        assert_eq!(proto.constant_count(), 3);
        assert_eq!(proto.max_stack_size(), 3);
    });
}

#[test]
fn double_not_literals_fold_to_one_cpp_boolean_load() {
    with_proto(
        br#"return not not nil, not not false, not not true, not not 1, not not ("value")"#,
        |proto| {
            let expected = [
                opcode::create_abc(OpCode::LOADBOOL, 0, 0, 0),
                opcode::create_abc(OpCode::LOADBOOL, 1, 0, 0),
                opcode::create_abc(OpCode::LOADBOOL, 2, 1, 0),
                opcode::create_abc(OpCode::LOADBOOL, 3, 1, 0),
                opcode::create_abc(OpCode::LOADBOOL, 4, 1, 0),
                opcode::create_abc(OpCode::RETURN, 0, 6, 0),
                opcode::create_abc(OpCode::RETURN, 0, 1, 0),
            ];

            assert_eq!(proto.code(), expected);
            assert_eq!(proto.line_info(), &[1; 7]);
            assert_eq!(proto.constant_count(), 0);
            assert_eq!(proto.max_stack_size(), 5);
        },
    );
}

#[test]
fn table_array_call_fields_reclaim_cpp_temporaries_between_slots() {
    with_proto(b"return {T.getref(d), T.getref(e), T.getref(f)}", |proto| {
        let expected = [
            opcode::create_abc(OpCode::NEWTABLE, 0, 2, 0),
            opcode::create_abx(OpCode::GETGLOBAL, 1, 0),
            opcode::create_abc(OpCode::GETTABLE, 2, 1, opcode::rk_ask(1)),
            opcode::create_abx(OpCode::GETGLOBAL, 3, 2),
            opcode::create_abc(OpCode::CALL, 2, 2, 2),
            opcode::create_abc(OpCode::MOVE, 1, 2, 0),
            opcode::create_abx(OpCode::GETGLOBAL, 2, 0),
            opcode::create_abc(OpCode::GETTABLE, 3, 2, opcode::rk_ask(1)),
            opcode::create_abx(OpCode::GETGLOBAL, 4, 3),
            opcode::create_abc(OpCode::CALL, 3, 2, 2),
            opcode::create_abc(OpCode::MOVE, 2, 3, 0),
            opcode::create_abx(OpCode::GETGLOBAL, 3, 0),
            opcode::create_abc(OpCode::GETTABLE, 4, 3, opcode::rk_ask(1)),
            opcode::create_abc(OpCode::MOVE, 3, 4, 0),
            opcode::create_abx(OpCode::GETGLOBAL, 4, 4),
            opcode::create_abc(OpCode::CALL, 3, 2, 0),
            opcode::create_abc(OpCode::SETLIST, 0, 0, 1),
            opcode::create_abc(OpCode::RETURN, 0, 2, 0),
            opcode::create_abc(OpCode::RETURN, 0, 1, 0),
        ];

        assert_eq!(proto.code(), expected);
        assert_eq!(proto.line_info(), &[1; 19]);
        assert_eq!(proto.constant_count(), 5);
        assert_eq!(proto.max_stack_size(), 5);
    });
}
