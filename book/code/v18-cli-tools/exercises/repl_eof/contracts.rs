use crate::model::{Finish, PendingInput};

// ANCHOR: repl_eof_contract
pub fn eof_reports_pending_once_and_releases_buffer(finish: Finish) {
    let mut idle = PendingInput::idle();
    assert!(finish(&mut idle).is_ok());
    assert!(idle.source.is_empty());
    for source in ["return 40 +\n", "if true then\n", "return [[unfinished\n"] {
        let mut pending = PendingInput::incomplete(source);
        let original = pending.diagnostic.clone().unwrap();
        let returned =
            finish(&mut pending).expect_err("pending EOF must report the original error");
        assert_eq!(returned, original); // Preserve kind, location and message.
        assert!(pending.source.is_empty());
        assert!(pending.diagnostic.is_none());
        assert!(
            finish(&mut pending).is_ok(),
            "EOF must not repeat its diagnostic"
        );
    }
}
// ANCHOR_END: repl_eof_contract
