use lua_core::Value;

use super::fixture::{exclusive_available, Fixture};
use super::model::{Close, FileError, FileVm};

pub fn releases_owner_keeps_identity(close: Close) {
    let fixture = Fixture::new();
    let path = fixture.path("contract.dat");
    let mut vm = FileVm::new();
    let (handle, dropped) = vm.register(fixture.open("contract.dat"));
    let alias = handle;
    assert_eq!(vm.io_type(alias), Some("file"));
    vm.write(alias, b"A\0\xff").unwrap();
    vm.flush(alias).unwrap();
    assert_eq!(vm.seek(alias), Ok(0));
    assert_eq!(vm.read(alias), Ok(b"A\0\xff".to_vec()));
    if let Some(available) = exclusive_available(&path) {
        assert!(!available);
    }
    // ANCHOR: file_close_contract
    assert_eq!(close(&mut vm, handle), Ok(()));
    assert_eq!(
        dropped.get(),
        1,
        "close must drop the unique File owner now"
    );
    assert_eq!(vm.io_type(alias), Some("closed file"));
    assert!(vm.files.get(&alias).is_some_and(Option::is_none));
    assert!(
        vm.claimed_closed.is_empty(),
        "no second boolean state is needed"
    );
    if let Some(available) = exclusive_available(&path) {
        assert!(available);
    }
    assert_eq!(close(&mut vm, alias), Err(FileError::AlreadyClosed));
    assert_eq!(vm.read(alias), Err(FileError::UseClosed));
    assert_eq!(vm.write(alias, b"x"), Err(FileError::UseClosed));
    assert_eq!(vm.seek(alias), Err(FileError::UseClosed));
    assert_eq!(vm.flush(alias), Err(FileError::UseClosed));
    assert_eq!(FileError::AlreadyClosed.kind(), "Runtime");
    assert_eq!(FileError::UseClosed.kind(), "Runtime");
    // ANCHOR_END: file_close_contract
    let foreign = vm.heap.allocate_userdata().unwrap();
    assert_eq!(vm.io_type(foreign), None);
    assert_eq!(close(&mut vm, foreign), Err(FileError::NotFile));
    assert_eq!(vm.read(foreign), Err(FileError::NotFile));
    assert_eq!(vm.write(foreign, b"x"), Err(FileError::NotFile));
    assert_eq!(vm.seek(foreign), Err(FileError::NotFile));
    assert_eq!(vm.flush(foreign), Err(FileError::NotFile));
    assert_eq!(FileError::NotFile.kind(), "Type");
    vm.heap.collect(&[Value::Userdata(alias)], &[]).unwrap();
    assert_eq!(vm.metadata_answer(alias), Value::Number(42.0));
    assert_eq!(Value::Userdata(handle), Value::Userdata(alias));
    drop(vm);
    assert_eq!(dropped.get(), 1, "closed tombstone must not drop twice");
    fixture.finish();
}
