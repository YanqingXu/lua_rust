use lua_vm::string_find::CaptureState;

/// A short mutable borrow of live slots and a read-only borrow of a real checkpoint.
pub type RestoreSlots = fn(&mut Vec<CaptureState>, &[CaptureState]);
