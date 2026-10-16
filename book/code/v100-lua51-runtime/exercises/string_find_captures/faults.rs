use lua_vm::string_find::CaptureState;

// ANCHOR: capture_restore_faults
pub fn truncate_only(slots: &mut Vec<CaptureState>, saved: &[CaptureState]) {
    slots.truncate(saved.len());
}

pub fn clear_everything(slots: &mut Vec<CaptureState>, saved: &[CaptureState]) {
    let _ = saved;
    slots.clear();
}

/// A design fault in a toy accounting projection, not a historical VM algorithm.
pub fn restore_work_counter(remaining: &mut usize, saved_remaining: usize) {
    *remaining = saved_remaining;
}
// ANCHOR_END: capture_restore_faults
