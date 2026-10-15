//! Fixed leader faults, supplied graph assembly, public DOT and real binary.
#[path = "../exercises/support/bytecode_process.rs"]
pub mod bytecode_process;
#[path = "../exercises/bytecode_cfg/contracts.rs"]
mod contracts;
#[path = "../exercises/bytecode_cfg/faults.rs"]
mod faults;
#[path = "../exercises/bytecode_cfg/fixtures.rs"]
mod fixtures;
#[path = "../exercises/bytecode_cfg/model.rs"]
pub mod model;
#[path = "../exercises/support/process.rs"]
pub mod process;
#[path = "../exercises/bytecode_cfg/real.rs"]
pub mod real;
#[path = "../exercises/bytecode_cfg/solution.rs"]
mod solution;

fn split() -> Result<(), String> {
    let case = fixtures::hidden_jump();
    let before = model::graph(&case.proto, faults::entry_and_targets)?;
    contracts::equal(
        "fixed missing split",
        before.blocks.clone(),
        vec![(0, 4), (4, 6)],
    )?;
    contracts::equal("fixed hidden jump lost", before.edges.clone(), vec![])?;
    println!(
        "before entry+target leaders={:?} graph={:?}",
        faults::entry_and_targets(&case.proto)?,
        model::graph(&case.proto, faults::entry_and_targets)?
    );
    contracts::leaders_split_control_flow_and_validate_instruction_targets(solution::leaders)?;
    println!(
        "after split leaders={:?} graph={:?}",
        solution::leaders(&case.proto)?,
        model::graph(&case.proto, solution::leaders)?
    );
    Ok(())
}

fn target() -> Result<(), String> {
    let case = fixtures::conditional(lua_core::Opcode::JumpIfFalse);
    let before = model::graph(&case.proto, faults::pc_relative);
    contracts::equal(
        "fixed wrong jump base",
        before.clone(),
        Err("pc 1 targets non-leader 3".to_owned()),
    )?;
    println!(
        "before pc+sBx leaders={:?} graph={before:?}",
        faults::pc_relative(&case.proto)?
    );
    println!(
        "after pc+1+sBx leaders={:?} graph={:?}",
        solution::leaders(&case.proto)?,
        model::graph(&case.proto, solution::leaders)?
    );
    Ok(())
}

fn lines() -> Result<(), String> {
    let case = fixtures::hidden_jump();
    contracts::equal(
        "fixed source line is not a block boundary",
        faults::source_lines(&case.proto)?,
        vec![0],
    )?;
    println!(
        "before same-source-line leaders={:?} graph={:?}",
        faults::source_lines(&case.proto)?,
        model::graph(&case.proto, faults::source_lines)?
    );
    println!(
        "after instruction boundaries leaders={:?}",
        solution::leaders(&case.proto)?
    );
    Ok(())
}

fn real() -> Result<(), String> {
    for (name, graph) in real::compiler_graphs(solution::leaders)? {
        println!("real compiler {name}: {graph:?}");
    }
    let binary = bytecode_process::example_binary()?;
    for (name, dot) in real::binary_at(&binary)? {
        println!("real binary {name}: exact DOT bytes={}", dot.len());
    }
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "split" => split(),
        "target" => target(),
        "lines" => lines(),
        "real" => real(),
        "all" => {
            split()?;
            target()?;
            lines()?;
            real()
        }
        _ => Err("usage: bytecode_cfg_problem_chain [split|target|lines|real|all]".to_owned()),
    }
}
