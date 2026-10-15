//! Fixed inconsistent adapters versus real Args; does not call learner starter.
#[path = "../exercises/args/faults.rs"]
mod faults;
#[path = "../exercises/args/real.rs"]
mod real;
#[path = "../exercises/args/solution.rs"]
mod solution;

use lua_core::{ByteString, Value};
use lua_stdlib::Args;

fn number() {
    let numeric_string = [
        Value::Boolean(false),
        Value::String(ByteString::new(b" 42.5 ".as_slice())),
    ];
    println!(
        "before numeric-string: {:?}",
        faults::strict_number(&numeric_string, 1)
    );
    println!(
        "after numeric-string: {:?}",
        solution::number(&numeric_string, 1)
    );
    for (label, values, index) in [
        ("missing", vec![], 0),
        ("nil", vec![Value::Nil], 0),
        ("boolean", vec![Value::Number(0.0), Value::Boolean(true)], 1),
    ] {
        println!(
            "before {label}: {:?}",
            faults::strict_number(&values, index)
        );
        println!("after {label}: {:?}", solution::number(&values, index));
    }
}

fn bytes() {
    let binary = [Value::String(ByteString::new(&[97, 0, 255, 122][..]))];
    println!("before binary: {:?}", faults::text_only_bytes(&binary, 0));
    println!("after binary: {:?}", solution::bytes(&binary, 0));
    println!(
        "before number-as-bytes: {:?}",
        faults::text_only_bytes(&[Value::Number(12.5)], 0)
    );
    println!(
        "after number-as-bytes: {:?}",
        solution::bytes(&[Value::Number(12.5)], 0)
    );
}

fn real() {
    let values = [Value::Number(7.0)];
    let borrowed = real::borrowed_value(Args::new(&values), 0).unwrap();
    println!(
        "real borrowed-alias: {}",
        std::ptr::eq(borrowed, &values[0])
    );
    println!(
        "real owned-after-drop: {:?}",
        real::owned_bytes_after_input_drops()
    );
    println!("real VM coercions: {:?}", real::vm_coercions());
    for (kind, message) in real::vm_argument_errors() {
        println!("real VM {kind:?}: {message}");
    }
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "number" => number(),
        "bytes" => bytes(),
        "real" => real(),
        "all" => {
            number();
            bytes();
            real();
        }
        _ => {
            eprintln!("usage: args_problem_chain [number|bytes|real|all]");
            std::process::exit(2);
        }
    }
}
