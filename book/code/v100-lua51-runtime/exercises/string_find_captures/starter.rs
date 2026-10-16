use lua_vm::string_find::CaptureState;

// ANCHOR: capture_restore_starter
pub fn restore_slots(slots: &mut Vec<CaptureState>, saved: &[CaptureState]) {
    // TODO: replace every live slot with the saved states, removing appended slots.
    let _ = (slots, saved);
}
// ANCHOR_END: capture_restore_starter
