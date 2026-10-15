use lua_core::UserdataRef;

use super::model::{FileError, FileVm};

// ANCHOR: file_close_flag_fault
pub fn flag_only(vm: &mut FileVm, handle: UserdataRef) -> Result<(), FileError> {
    let entry = vm.files.get(&handle).ok_or(FileError::NotFile)?;
    if entry.is_none() || !vm.claimed_closed.insert(handle) {
        return Err(FileError::AlreadyClosed);
    }
    // Claimed closed, but Some(OwnedFile) still owns the OS handle.
    Ok(())
}
// ANCHOR_END: file_close_flag_fault

// ANCHOR: file_close_remove_fault
pub fn remove_entry(vm: &mut FileVm, handle: UserdataRef) -> Result<(), FileError> {
    let owned = vm.files.remove(&handle).ok_or(FileError::NotFile)?;
    drop(owned.ok_or(FileError::AlreadyClosed)?);
    Ok(())
}
// ANCHOR_END: file_close_remove_fault
