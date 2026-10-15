use crate::model::{edge, Graph};
use lua_core::{Instruction, Opcode, Proto};

pub fn plain(opcode: Opcode) -> Instruction {
    Instruction::abc(opcode, 0, 0, 0)
}

pub fn proto(code: Vec<Instruction>) -> Proto {
    Proto {
        lines: vec![1; code.len()],
        code,
        constants: vec![],
        max_stack_size: 4,
        protos: vec![],
        num_params: 0,
        is_vararg: false,
        upvalues: vec![],
        debug_name: "cfg-teaching".to_owned(),
    }
}

pub struct Case {
    pub name: &'static str,
    pub proto: Proto,
    pub leaders: Vec<usize>,
    pub graph: Graph,
}

// ANCHOR: cfg_fixture_control
pub fn hidden_jump() -> Case {
    Case {
        name: "unreachable instructions after Jump",
        proto: proto(vec![
            plain(Opcode::Move),
            Instruction::asbx(Opcode::Jump, 0, 2),
            plain(Opcode::Move),
            plain(Opcode::Return),
            plain(Opcode::Move),
            plain(Opcode::Return),
        ]),
        leaders: vec![0, 2, 4],
        graph: Graph {
            blocks: vec![(0, 2), (2, 4), (4, 6)],
            edges: vec![edge(0, 2, "jump")],
        },
    }
}

pub fn conditional(opcode: Opcode) -> Case {
    Case {
        name: "conditional jump and fallthrough",
        proto: proto(vec![
            plain(Opcode::Move),
            Instruction::asbx(opcode, 0, 1),
            plain(Opcode::Move),
            plain(Opcode::Return),
        ]),
        leaders: vec![0, 2, 3],
        graph: Graph {
            blocks: vec![(0, 2), (2, 3), (3, 4)],
            edges: vec![edge(0, 2, "jump"), edge(0, 1, "next"), edge(1, 2, "next")],
        },
    }
}
// ANCHOR_END: cfg_fixture_control

pub fn matrix() -> Vec<Case> {
    let mut cases = vec![hidden_jump()];
    for opcode in [Opcode::JumpIfFalse, Opcode::JumpIfTrue, Opcode::JumpIfNil] {
        cases.push(conditional(opcode));
    }
    cases.extend([
        Case {
            name: "forward and backward jumps",
            proto: proto(vec![
                Instruction::asbx(Opcode::Jump, 0, 3),
                plain(Opcode::Move),
                Instruction::asbx(Opcode::Jump, 0, -3),
                plain(Opcode::Return),
                plain(Opcode::Move),
                plain(Opcode::Return),
            ]),
            leaders: vec![0, 1, 3, 4],
            graph: Graph {
                blocks: vec![(0, 1), (1, 3), (3, 4), (4, 6)],
                edges: vec![edge(0, 3, "jump"), edge(1, 0, "jump")],
            },
        },
        Case {
            name: "ForPrep jump only, ForLoop jump and next",
            proto: proto(vec![
                Instruction::asbx(Opcode::ForPrep, 0, 1),
                plain(Opcode::Move),
                Instruction::asbx(Opcode::ForLoop, 0, -2),
                plain(Opcode::Return),
            ]),
            leaders: vec![0, 1, 2, 3],
            graph: Graph {
                blocks: vec![(0, 1), (1, 2), (2, 3), (3, 4)],
                edges: vec![
                    edge(0, 2, "jump"),
                    edge(1, 2, "next"),
                    edge(2, 1, "jump"),
                    edge(2, 3, "next"),
                ],
            },
        },
        Case {
            name: "Return and TailCall terminate blocks",
            proto: proto(vec![
                plain(Opcode::Return),
                plain(Opcode::Move),
                plain(Opcode::TailCall),
                plain(Opcode::Move),
                plain(Opcode::Return),
            ]),
            leaders: vec![0, 1, 3],
            graph: Graph {
                blocks: vec![(0, 1), (1, 3), (3, 5)],
                edges: vec![],
            },
        },
        Case {
            name: "empty code",
            proto: proto(vec![]),
            leaders: vec![],
            graph: Graph {
                blocks: vec![],
                edges: vec![],
            },
        },
        Case {
            name: "comparison produces a value, not a branch",
            proto: proto(vec![
                plain(Opcode::Eq),
                plain(Opcode::Lt),
                plain(Opcode::Return),
            ]),
            leaders: vec![0],
            graph: Graph {
                blocks: vec![(0, 3)],
                edges: vec![],
            },
        },
    ]);
    cases
}

pub fn invalid() -> Vec<(Proto, String)> {
    vec![
        (
            proto(vec![
                plain(Opcode::Move),
                Instruction::from_raw(63),
                plain(Opcode::Return),
            ]),
            "invalid opcode 63 at pc 1".to_owned(),
        ),
        (
            proto(vec![Instruction::asbx(Opcode::Jump, 0, -2)]),
            "jump at pc 0 targets -1 outside code".to_owned(),
        ),
        (
            proto(vec![Instruction::asbx(Opcode::Jump, 0, 0)]),
            "jump at pc 0 targets 1 outside code".to_owned(),
        ),
        (
            proto(vec![
                plain(Opcode::Move),
                Instruction::asbx(Opcode::JumpIfFalse, 0, 3),
            ]),
            "jump at pc 1 targets 5 outside code".to_owned(),
        ),
    ]
}
