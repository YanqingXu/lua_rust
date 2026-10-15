//! 固定反例与参考清理，完全不读取 learner starter。
#[path = "../exercises/file_prune/contracts.rs"]
mod contracts;
#[path = "../exercises/file_prune/faults.rs"]
mod faults;
#[path = "../exercises/file_prune/model.rs"]
mod model;
#[path = "../exercises/file_prune/real.rs"]
mod real;
#[path = "../exercises/file_prune/solution.rs"]
mod solution;
#[path = "../exercises/file_prune/temp.rs"]
mod temp;

fn gap() -> Result<(), String> {
    println!(
        "before missing prune: {:?}",
        contracts::missing_prune(faults::forget_prune)?
    );
    println!(
        "before Option::is_some: {:?}",
        contracts::option_is_not_liveness(faults::prune_closed_instead_of_dead)?
    );
    Ok(())
}

fn prune() -> Result<(), String> {
    println!(
        "incomplete Sweep: {:?}",
        contracts::incomplete_cycle_keeps_map(solution::prune)?
    );
    let [gap, pruned, released] =
        contracts::completed_cycle_prunes_dead_userdata_and_keeps_live_tombstones(solution::prune)?;
    println!("completed before retain: {gap:?}");
    println!("completed after retain: {pruned:?}");
    println!("after removing roots: {released:?}");
    Ok(())
}

fn roots() -> Result<(), String> {
    println!(
        "before rooting map keys: {:?}",
        contracts::root_all_keys_leaks(solution::prune)?
    );
    Ok(())
}

fn real() -> Result<(), String> {
    println!("real VM: {:?}", real::vm_weak_slots_and_os_release()?);
    Ok(())
}

fn main() -> Result<(), String> {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "gap" => gap(),
        "prune" => prune(),
        "roots" => roots(),
        "real" => real(),
        "all" => {
            gap()?;
            prune()?;
            roots()?;
            real()
        }
        _ => Err("usage: file_prune_problem_chain [gap|prune|roots|real|all]".to_owned()),
    }
}
