use lua_vm::string_find::CaptureState;

// ANCHOR: capture_restore_solution
pub fn restore_slots(slots: &mut Vec<CaptureState>, saved: &[CaptureState]) {
    slots.clear();
    slots.extend_from_slice(saved);
}
// ANCHOR_END: capture_restore_solution
