use crate::model::{Finish, PendingInput};

// ANCHOR: repl_migration_contract
pub fn migrated_eof_consumes_original_diagnostic_once(finish: Finish) {
    let mut idle = PendingInput::idle();
    assert!(finish(&mut idle).is_ok());
    for source in ["return 40 +\n", "if true then\n", "return [=[unfinished\n"] {
        let mut input = PendingInput::incomplete(source);
        let original = input.diagnostic.clone().unwrap();
        assert_eq!(
            finish(&mut input).expect_err("pending EOF is not successful input"),
            original,
            "preserve kind, position and message from the local compiler"
        );
        assert!(input.source.is_empty());
        assert!(input.diagnostic.is_none());
        assert!(
            finish(&mut input).is_ok(),
            "the error must be consumed once"
        );
    }
}
// ANCHOR_END: repl_migration_contract
