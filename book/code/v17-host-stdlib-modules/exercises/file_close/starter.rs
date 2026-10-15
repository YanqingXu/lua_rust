use lua_core::UserdataRef;

use super::model::{FileError, FileVm};

// ANCHOR: file_close_starter
pub fn close(vm: &mut FileVm, handle: UserdataRef) -> Result<(), FileError> {
    // TODO: release the File while preserving this userdata's None tombstone.
    let owned = vm.files.remove(&handle).ok_or(FileError::NotFile)?;
    drop(owned.ok_or(FileError::AlreadyClosed)?);
    Ok(())
}
// ANCHOR_END: file_close_starter
