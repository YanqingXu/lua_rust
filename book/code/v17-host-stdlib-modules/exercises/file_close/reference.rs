mod contracts;
mod faults;
mod fixture;
mod model;
mod real;
mod solution;

use lua_core::Value;
use model::{FileError, FileVm};

#[test]
fn close_releases_owner_but_keeps_closed_identity() {
    contracts::releases_owner_keeps_identity(solution::close);
}

#[test]
fn flag_only_keeps_the_file_owner_alive() {
    let fixture = fixture::Fixture::new();
    let path = fixture.path("flag.dat");
    let mut vm = FileVm::new();
    let (handle, dropped) = vm.register(fixture.open("flag.dat"));
    faults::flag_only(&mut vm, handle).unwrap();
    assert!(vm.claimed_closed.contains(&handle));
    assert_eq!(vm.io_type(handle), Some("file"));
    assert_eq!(dropped.get(), 0);
    if let Some(available) = fixture::exclusive_available(&path) {
        assert!(!available);
    }
    drop(vm);
    assert_eq!(dropped.get(), 1);
    fixture.finish();
}

#[test]
fn removing_entry_loses_closed_file_classification() {
    let fixture = fixture::Fixture::new();
    let mut vm = FileVm::new();
    let (handle, dropped) = vm.register(fixture.open("remove.dat"));
    faults::remove_entry(&mut vm, handle).unwrap();
    assert_eq!(dropped.get(), 1);
    assert_eq!(vm.io_type(handle), None);
    assert_eq!(
        faults::remove_entry(&mut vm, handle),
        Err(FileError::NotFile)
    );
    assert_eq!(vm.metadata_answer(handle), Value::Number(42.0));
    drop(vm);
    fixture.finish();
}

#[test]
fn model_closed_userdata_identity_and_metadata_survive_collection() {
    let fixture = fixture::Fixture::new();
    let mut vm = FileVm::new();
    let (handle, dropped) = vm.register(fixture.open("gc.dat"));
    solution::close(&mut vm, handle).unwrap();
    let alias = Value::Userdata(handle);
    vm.heap.collect(std::slice::from_ref(&alias), &[]).unwrap();
    assert_eq!(alias, Value::Userdata(handle));
    assert_eq!(vm.metadata_answer(handle), Value::Number(42.0));
    assert_eq!(dropped.get(), 1);
    drop(vm);
    fixture.finish();
}

#[test]
fn real_vm_io_close_and_file_close_share_alias_state() {
    assert_eq!(
        real::both_close_routes_share_alias_state(),
        Value::Boolean(true)
    );
}

#[test]
fn real_vm_closed_operations_are_runtime_errors_and_wrong_types_are_type_errors() {
    let probes = real::closed_and_wrong_type_error_matrix();
    assert_eq!(probes.len(), 12);
    for probe in probes {
        assert!(!probe.operation.is_empty() && !probe.message.is_empty());
        assert!(matches!(
            probe.kind,
            lua_vm::VmErrorKind::Type | lua_vm::VmErrorKind::Runtime
        ));
    }
}

#[test]
fn real_vm_closed_userdata_retains_its_metatable_edge_across_execute_and_gc() {
    assert_eq!(
        real::closed_userdata_keeps_metatable_edge_after_gc(),
        Value::Boolean(true)
    );
}

#[test]
fn real_vm_windows_exclusive_reopen_observes_file_release() {
    let (before, after) = real::exclusive_reopen_before_and_after_close();
    #[cfg(windows)]
    assert_eq!((before, after), (Some(false), Some(true)));
    #[cfg(not(windows))]
    assert_eq!((before, after), (None, None));
}
