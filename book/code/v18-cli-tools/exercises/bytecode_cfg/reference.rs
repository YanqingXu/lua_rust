#[path = "../support/bytecode_process.rs"]
pub mod bytecode_process;
mod contracts;
mod faults;
mod fixtures;
pub mod model;
#[path = "../support/process.rs"]
pub mod process;
pub mod real;
mod solution;

use lua_core::{Instruction, Opcode};
use std::path::Path;
use std::rc::Rc;

#[test]
fn leaders_split_control_flow_and_validate_instruction_targets() {
    contracts::leaders_split_control_flow_and_validate_instruction_targets(solution::leaders)
        .unwrap();
}

#[test]
fn faulty_entry_and_targets_hide_jump_inside_a_block() {
    let case = fixtures::hidden_jump();
    assert_eq!(faults::entry_and_targets(&case.proto), Ok(vec![0, 4]));
    let graph = model::graph(&case.proto, faults::entry_and_targets).unwrap();
    assert_eq!(graph.blocks, vec![(0, 4), (4, 6)]);
    assert_eq!(graph.edges, vec![]);
    assert_eq!(case.graph.edges, vec![model::edge(0, 2, "jump")]);
}

#[test]
fn faulty_pc_relative_target_uses_wrong_base() {
    let case = fixtures::conditional(Opcode::JumpIfFalse);
    assert_eq!(faults::pc_relative(&case.proto), Ok(vec![0, 2]));
    assert_eq!(
        model::graph(&case.proto, faults::pc_relative),
        Err("pc 1 targets non-leader 3".to_owned())
    );
    assert_eq!(model::graph(&case.proto, solution::leaders), Ok(case.graph));
}

#[test]
fn faulty_source_lines_do_not_define_basic_blocks() {
    let mut case = fixtures::hidden_jump();
    assert_eq!(faults::source_lines(&case.proto), Ok(vec![0]));
    assert_eq!(
        model::graph(&case.proto, faults::source_lines)
            .unwrap()
            .edges,
        vec![]
    );
    case.proto.lines = (100..100 + case.proto.code.len()).collect();
    assert_eq!(solution::leaders(&case.proto), Ok(case.leaders));
    assert_eq!(model::graph(&case.proto, solution::leaders), Ok(case.graph));
    let mut straight = fixtures::proto(vec![
        fixtures::plain(Opcode::Move),
        fixtures::plain(Opcode::Move),
        fixtures::plain(Opcode::Return),
    ]);
    straight.lines = vec![1, 2, 3];
    assert_eq!(faults::source_lines(&straight), Ok(vec![0, 1, 2]));
    assert_eq!(solution::leaders(&straight), Ok(vec![0]));
}

#[test]
fn public_cfg_keeps_unreachable_blocks_and_ignores_nested_protos() {
    let case = fixtures::hidden_jump();
    assert_eq!(
        model::graph(&case.proto, solution::leaders),
        Ok(case.graph.clone())
    );
    assert_eq!(case.graph.blocks[1], (2, 4)); // No edge reaches this block; it remains present.
    let mut outer = fixtures::proto(vec![fixtures::plain(Opcode::Return)]);
    outer
        .protos
        .push(Rc::new(fixtures::proto(vec![Instruction::from_raw(63)])));
    let expected = model::Graph {
        blocks: vec![(0, 1)],
        edges: vec![],
    };
    assert_eq!(
        model::graph(&outer, solution::leaders),
        Ok(expected.clone())
    );
    assert_eq!(lua_bytecode::cfg_dot(&outer), expected.dot(&outer));
}

#[test]
fn real_compiler_if_and_for_match_exact_blocks_and_edges() {
    assert_eq!(real::compiler_graphs(solution::leaders).unwrap().len(), 2);
}

#[test]
fn real_binary_cfg_matches_public_dot_for_compiled_sources() {
    assert_eq!(
        real::binary_graphs(Path::new(env!("CARGO_BIN_EXE_lua_app")))
            .unwrap()
            .len(),
        2
    );
}
