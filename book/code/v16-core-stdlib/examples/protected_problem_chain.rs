//! 固定重放结果协议，不读取 learner starter。
#[path = "../exercises/protected/faults.rs"]
mod faults;
#[path = "../exercises/protected/model.rs"]
pub mod model;
#[path = "../exercises/protected/real.rs"]
mod real;
#[path = "../exercises/protected/solution.rs"]
mod solution;

use lua_core::Value::{Nil, Number};

fn pcall() {
    let values = vec![Number(7.0), Nil, Number(9.0), Nil];
    println!(
        "before pcall first-only={:?}",
        faults::first_result_only(Ok(values.clone()))
    );
    println!(
        "after pcall complete={:?}",
        solution::pack_pcall(Ok(values))
    );
    println!("after pcall empty={:?}", solution::pack_pcall(Ok(vec![])));
    let failed = solution::pack_pcall(Err(model::body_error()));
    println!("after pcall error: ok={} message={}", failed[0], failed[1]);
}

fn xpcall() {
    for (label, pack) in [
        (
            "before always-handler",
            faults::always_handle_and_forward_all as model::XPacker,
        ),
        ("after conditional", solution::pack_xpcall as model::XPacker),
    ] {
        let mut calls = 0;
        let values = pack(Ok(vec![Number(7.0), Nil, Number(9.0)]), &mut |_| {
            calls += 1;
            Ok(vec![Number(42.0), Number(99.0)])
        });
        println!("{label}: calls={calls} values={values:?}");
        let values = pack(Err(model::body_error()), &mut |_| {
            Ok(vec![Number(42.0), Number(99.0)])
        });
        println!("{label} body-error={values:?}");
    }
    println!(
        "after handler-empty={:?}",
        solution::pack_xpcall(Err(model::body_error()), &mut |_| Ok(vec![]))
    );
    let failed = solution::pack_xpcall(Err(model::body_error()), &mut |_| {
        Err(model::handler_error())
    });
    println!(
        "after handler-error: ok={} message={}",
        failed[0], failed[1]
    );
}

fn real() {
    println!("real pcall count+values={:?}", real::pcall_multi_values());
    println!(
        "real same-execute caught/later-ok/value/prefix/weak-nil={:?}",
        real::run(real::ERROR_THEN_CONTINUE)
    );
    println!(
        "real xpcall success+calls={:?}",
        real::xpcall_success_skips_handler()
    );
    println!("real xpcall variants={:?}", real::xpcall_handler_variants());
    println!(
        "real formatted-errors={:?}",
        real::errors_are_formatted_strings()
    );
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "pcall" => pcall(),
        "xpcall" => xpcall(),
        "real" => real(),
        "all" => {
            pcall();
            xpcall();
            real();
        }
        _ => {
            eprintln!("usage: protected_problem_chain [pcall|xpcall|real|all]");
            std::process::exit(2);
        }
    }
}
