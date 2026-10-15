use lua_core::UserdataRef;

use super::model::{FileError, FileVm};

// ANCHOR: file_close_solution
pub fn close(vm: &mut FileVm, handle: UserdataRef) -> Result<(), FileError> {
    let entry = vm.files.get_mut(&handle).ok_or(FileError::NotFile)?;
    let owned = entry.take().ok_or(FileError::AlreadyClosed)?;
    drop(owned);
    Ok(())
}
// ANCHOR_END: file_close_solution
