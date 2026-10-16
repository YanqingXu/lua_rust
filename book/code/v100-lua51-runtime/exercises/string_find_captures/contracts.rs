use crate::model::RestoreSlots;
use lua_vm::string_find::{capture_checkpoint, CaptureState};

// ANCHOR: capture_restore_contract
pub fn checkpoint_restore_replaces_old_slots_and_removes_new_slots(restore: RestoreSlots) {
    for (original, mut live) in [
        (vec![], vec![CaptureState::Open(0)]),
        (
            vec![CaptureState::Open(0)],
            vec![CaptureState::Closed(0..3)],
        ),
        (
            vec![
                CaptureState::Open(1),
                CaptureState::Closed(2..4),
                CaptureState::Position(5),
            ],
            vec![
                CaptureState::Closed(1..7),
                CaptureState::Open(2),
                CaptureState::Position(8),
                CaptureState::Open(9),
            ],
        ),
    ] {
        let checkpoint = capture_checkpoint(&original).unwrap();
        restore(&mut live, checkpoint.states());
        assert_eq!(live, original, "restore contents, not only slot count");
        assert_eq!(
            checkpoint.states(),
            original,
            "the borrowed checkpoint is unchanged"
        );
        // A checkpoint is reusable; rolling back must not consume its contents.
        live.push(CaptureState::Position(10));
        restore(&mut live, checkpoint.states());
        assert_eq!(live, original);
    }
}
// ANCHOR_END: capture_restore_contract
