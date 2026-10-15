//! Real Proto/Instruction inputs; fixed graph assembly isolates leader discovery.
use lua_core::{Opcode, Proto};
use std::fmt::Write as _;

pub type Leaders = fn(&Proto) -> Result<Vec<usize>, String>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: &'static str,
}

pub fn edge(from: usize, to: usize, label: &'static str) -> Edge {
    Edge { from, to, label }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Graph {
    pub blocks: Vec<(usize, usize)>,
    pub edges: Vec<Edge>,
}

// ANCHOR: cfg_flow_classes
pub fn is_jump(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::Jump
            | Opcode::JumpIfFalse
            | Opcode::JumpIfTrue
            | Opcode::JumpIfNil
            | Opcode::ForPrep
            | Opcode::ForLoop
    )
}

pub fn has_fallthrough(opcode: Opcode) -> bool {
    !matches!(
        opcode,
        Opcode::Return | Opcode::TailCall | Opcode::Jump | Opcode::ForPrep
    )
}
// ANCHOR_END: cfg_flow_classes

// ANCHOR: cfg_jump_target
pub fn target(pc: usize, sbx: i32, len: usize) -> Result<usize, String> {
    let target = pc as i64 + 1 + i64::from(sbx);
    if target < 0 || target >= len as i64 {
        return Err(format!("jump at pc {pc} targets {target} outside code"));
    }
    Ok(target as usize)
}
// ANCHOR_END: cfg_jump_target

// ANCHOR: cfg_block_slices
pub fn graph(proto: &Proto, discover: Leaders) -> Result<Graph, String> {
    let starts = discover(proto)?;
    if proto.code.is_empty() {
        return if starts.is_empty() {
            Ok(Graph {
                blocks: vec![],
                edges: vec![],
            })
        } else {
            Err("empty code must have no leaders".to_owned())
        };
    }
    if starts.first() != Some(&0)
        || starts.iter().any(|pc| *pc >= proto.code.len())
        || starts.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err("leaders must start at 0 and be sorted, unique, in range".to_owned());
    }
    let blocks = starts
        .iter()
        .copied()
        .enumerate()
        .map(|(index, start)| {
            (
                start,
                starts.get(index + 1).copied().unwrap_or(proto.code.len()),
            )
        })
        .collect::<Vec<_>>();
    let edges = edges(proto, &blocks)?;
    Ok(Graph { blocks, edges })
}
// ANCHOR_END: cfg_block_slices

// ANCHOR: cfg_edges
fn edges(proto: &Proto, blocks: &[(usize, usize)]) -> Result<Vec<Edge>, String> {
    let mut edges = Vec::new();
    for (index, (_, end)) in blocks.iter().copied().enumerate() {
        let pc = end - 1; // Only the last instruction determines outgoing edges.
        let instruction = proto.code[pc];
        let opcode = instruction
            .opcode()
            .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
        let mut targets = Vec::new();
        if is_jump(opcode) {
            targets.push((target(pc, instruction.sbx(), proto.code.len())?, "jump"));
        }
        if has_fallthrough(opcode) && end < proto.code.len() {
            targets.push((end, "next"));
        }
        for (target, label) in targets {
            let to = blocks
                .iter()
                .position(|(start, _)| *start == target)
                .ok_or_else(|| format!("pc {pc} targets non-leader {target}"))?;
            edges.push(edge(index, to, label));
        }
    }
    Ok(edges)
}
// ANCHOR_END: cfg_edges

impl Graph {
    pub fn dot(&self, proto: &Proto) -> Result<String, String> {
        let mut output =
            String::from("digraph bytecode {\n  node [shape=box,fontname=monospace];\n");
        for (index, (start, end)) in self.blocks.iter().copied().enumerate() {
            let mut label = format!("B{index}\\n");
            for pc in start..end {
                let opcode = proto.code[pc]
                    .opcode()
                    .map_err(|raw| format!("invalid opcode {raw} at pc {pc}"))?;
                let _ = write!(label, "{pc}: {opcode:?}\\l");
            }
            let _ = writeln!(output, "  b{index} [label=\"{label}\"];");
        }
        for edge in &self.edges {
            let _ = writeln!(
                output,
                "  b{} -> b{} [label=\"{}\"];",
                edge.from, edge.to, edge.label
            );
        }
        output.push_str("}\n");
        Ok(output)
    }
}
