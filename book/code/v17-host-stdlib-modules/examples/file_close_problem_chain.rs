//! Fixed ownership faults and reference; this replay never reads learner starter.
#[path = "../exercises/file_close/faults.rs"]
mod faults;
#[path = "../exercises/file_close/fixture.rs"]
mod fixture;
#[path = "../exercises/file_close/model.rs"]
mod model;
#[path = "../exercises/file_close/real.rs"]
mod real;
#[path = "../exercises/file_close/solution.rs"]
mod solution;

use lua_core::Value;
use model::{Close, FileError, FileVm};

fn close() {
    for (label, hook) in [
        ("before flag-only", faults::flag_only as Close),
        ("before remove-entry", faults::remove_entry as Close),
        ("after take-owner", solution::close as Close),
    ] {
        let fixture = fixture::Fixture::new();
        let path = fixture.path("replay.dat");
        let mut vm = FileVm::new();
        let (handle, dropped) = vm.register(fixture.open("replay.dat"));
        vm.write(handle, b"A\0\xff").unwrap();
        vm.flush(handle).unwrap();
        vm.seek(handle).unwrap();
        assert_eq!(vm.read(handle).unwrap(), b"A\0\xff");
        let before = fixture::exclusive_available(&path);
        let result = hook(&mut vm, handle);
        let after = fixture::exclusive_available(&path);
        let repeated = hook(&mut vm, handle);
        println!("{label}: close={result:?}, io.type={:?}, drops={}, claimed={}, repeat={repeated:?}, exclusive={before:?}->{after:?}",vm.io_type(handle),dropped.get(),vm.claimed_closed.contains(&handle));
        drop(vm);
        fixture.finish();
    }
}

fn identity() {
    let fixture = fixture::Fixture::new();
    let mut vm = FileVm::new();
    let (handle, dropped) = vm.register(fixture.open("identity.dat"));
    let alias = Value::Userdata(handle);
    solution::close(&mut vm, handle).unwrap();
    vm.heap.collect(std::slice::from_ref(&alias), &[]).unwrap();
    let metadata = vm.metadata_answer(handle);
    assert_eq!(metadata, Value::Number(42.0));
    let error = vm.read(handle).unwrap_err();
    assert_eq!(error, FileError::UseClosed);
    println!(
        "after GC: same={}, io.type={:?}, metadata={metadata}, drops={}, read-error={}",
        alias == Value::Userdata(handle),
        vm.io_type(handle),
        dropped.get(),
        error.kind()
    );
    drop(vm);
    fixture.finish();
}

fn real() {
    let aliases = real::both_close_routes_share_alias_state();
    assert_eq!(aliases, Value::Boolean(true));
    println!("real close routes + aliases={aliases}");
    for probe in real::closed_and_wrong_type_error_matrix() {
        println!(
            "real {}: {:?}: {}",
            probe.operation, probe.kind, probe.message
        );
    }
    let metadata = real::closed_userdata_keeps_metatable_edge_after_gc();
    assert_eq!(metadata, Value::Boolean(true));
    println!("real closed userdata metadata after GC={metadata}");
    let (before, after) = real::exclusive_reopen_before_and_after_close();
    match (before, after) {
        (Some(before), Some(after)) => {
            println!("real Windows exclusive reopen: before={before}, after={after}")
        }
        (None, None) => println!(
            "real exclusive reopen: unsupported on this platform; portable contracts still run"
        ),
        _ => unreachable!(),
    }
}

fn main() {
    match std::env::args().nth(1).as_deref().unwrap_or("all") {
        "close" => close(),
        "identity" => identity(),
        "real" => real(),
        "all" => {
            close();
            identity();
            real();
        }
        _ => {
            eprintln!("usage: file_close_problem_chain [close|identity|real|all]");
            std::process::exit(2);
        }
    }
}
