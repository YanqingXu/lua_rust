//! Fixed Session ownership comparison; never reads the learner starter.
#[path = "../exercises/repl_owner/chunks.rs"]
mod chunks;
#[path = "../exercises/repl_owner/contracts.rs"]
mod contracts;
#[path = "../exercises/repl_owner/faults.rs"]
mod faults;
#[path = "../exercises/support/process.rs"]
pub mod process;
#[path = "../exercises/repl_owner/real.rs"]
mod real;
#[path = "../exercises/repl_owner/solution.rs"]
mod solution;
#[path = "../exercises/support/temp.rs"]
pub mod temp;

fn owner() -> Result<(), String> {
    let [setup, lost] = chunks::observe_pair(faults::fresh_session)?;
    contracts::equal("fixed fresh setup", setup.clone(), chunks::setup_values())?;
    contracts::equal(
        "fixed fresh loses all seven fields",
        lost.clone(),
        vec![lua_core::Value::Nil; 7],
    )?;
    println!("before fresh setup={setup:?}");
    println!("before fresh next+GC={lost:?}");
    let [setup, observed, advanced] =
        contracts::submissions_preserve_globals_tables_closures_and_owner_edges(solution::submit)?;
    println!("after shared setup={setup:?}");
    println!("after shared next+GC={observed:?}");
    println!("after shared mutation+GC={advanced:?}");
    Ok(())
}

fn roots() -> Result<(), String> {
    println!(
        "real Vm host roots: {:?}",
        real::copied_host_handle_is_not_a_root()?
    );
    Ok(())
}

fn cli() -> Result<(), String> {
    let binary = process::example_binary()?;
    println!(
        "real CLI -e/-e/script: {:?}",
        real::cli_multiple_actions_then_script(&binary)?
    );
    println!(
        "real CLI -e/-i inputs: {:?}",
        real::cli_actions_then_interactive_inputs(&binary)?
    );
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "owner" => owner(),
        "roots" => roots(),
        "cli" => cli(),
        "all" => {
            owner()?;
            roots()?;
            cli()
        }
        _ => Err("usage: repl_owner_problem_chain [owner|roots|cli|all]".to_owned()),
    }
}
