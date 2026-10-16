use crate::{faults, solution};
use lua_app::{execute_multi, AppError, Session};
use lua_core::{ByteString, Value};
use lua_vm::string_find::{
    capture_checkpoint, find_captures, find_captures_with_limits, restore_captures, Capture,
    CaptureState, FindError, FindMatch, Limits,
};
use lua_vm::VmErrorKind;

pub fn truncate_or_clear_cannot_restore_mutated_existing_slots() {
    let original = vec![CaptureState::Open(0), CaptureState::Position(1)];
    let checkpoint = capture_checkpoint(&original).unwrap();
    let changed = vec![
        CaptureState::Closed(0..3),
        CaptureState::Position(4),
        CaptureState::Open(5),
    ];
    let mut truncated = changed.clone();
    faults::truncate_only(&mut truncated, checkpoint.states());
    assert_eq!(truncated.len(), original.len());
    assert_ne!(
        truncated, original,
        "correct length does not restore old slot contents"
    );
    let mut cleared = changed;
    faults::clear_everything(&mut cleared, checkpoint.states());
    assert!(cleared.is_empty());
    assert_ne!(
        cleared, original,
        "clearing also loses captures opened before the retry"
    );
}

pub fn restoring_a_work_counter_refunds_work_that_was_already_spent() {
    let checkpoint = capture_checkpoint(&[CaptureState::Open(0)]).unwrap();
    let saved_work = 8;
    let mut remaining = saved_work - 3;
    let mut slots = vec![CaptureState::Closed(0..2)];
    solution::restore_slots(&mut slots, checkpoint.states());
    assert_eq!(remaining, 5); // The hook has no borrow of this separate owner.
    faults::restore_work_counter(&mut remaining, saved_work);
    assert_eq!(
        remaining, 8,
        "the design fault incorrectly refunds three work units"
    );
}

// ANCHOR: capture_real_retry
pub fn actual_backtracking_restores_capture_ends_before_retrying_the_suffix() {
    assert_eq!(
        find_captures(b"aaab", b"(a*)(a)b", 1, false).unwrap(),
        Some(FindMatch {
            span: 0..4,
            captures: vec![Capture::Bytes(0..2), Capture::Bytes(2..3)]
        })
    );
    assert_eq!(
        find_captures(b"aaab", b"(a*)ab", 1, false).unwrap(),
        Some(FindMatch {
            span: 0..4,
            captures: vec![Capture::Bytes(0..2)]
        })
    );
    assert_eq!(
        find_captures(b"ab", b"(a*)ab", 1, false).unwrap(),
        Some(FindMatch {
            span: 0..2,
            captures: vec![Capture::Bytes(0..0)]
        })
    );
    assert_eq!(find_captures(b"ab", b"(a+)ab", 1, false).unwrap(), None);
    assert_eq!(
        execute_multi(b"return string.find('aaab','(a*)(a)b')").unwrap(),
        [
            Value::Number(1.0),
            Value::Number(4.0),
            Value::String(ByteString::new(b"aa".as_slice())),
            Value::String(ByteString::new(b"a".as_slice()))
        ]
    );
}
// ANCHOR_END: capture_real_retry

// ANCHOR: capture_real_kinds
pub fn actual_nested_empty_and_position_captures_keep_distinct_kinds() {
    assert_eq!(
        find_captures(b"xxabyy", b"((a)b)", 1, false).unwrap(),
        Some(FindMatch {
            span: 2..4,
            captures: vec![Capture::Bytes(2..4), Capture::Bytes(2..3)]
        })
    );
    assert_eq!(
        find_captures(b"aaa", b"()(a*)()", 1, false).unwrap(),
        Some(FindMatch {
            span: 0..3,
            captures: vec![
                Capture::Position(0),
                Capture::Bytes(0..3),
                Capture::Position(3)
            ]
        })
    );
    assert_eq!(
        execute_multi(b"return string.find('b','(a*)')").unwrap(),
        [
            Value::Number(1.0),
            Value::Number(0.0),
            Value::String(ByteString::new(b"".as_slice()))
        ]
    );
    assert_eq!(
        execute_multi(b"return string.find('b','()')").unwrap(),
        [Value::Number(1.0), Value::Number(0.0), Value::Number(1.0)]
    );
}
// ANCHOR_END: capture_real_kinds

pub fn actual_candidate_and_next_find_start_with_independent_capture_slots() {
    assert_eq!(
        find_captures(b"aacaaab", b"(a*)b", 1, false).unwrap(),
        Some(FindMatch {
            span: 3..7,
            captures: vec![Capture::Bytes(3..6)]
        })
    );
    assert_eq!(
        find_captures(b"z", b"()", 1, false).unwrap(),
        Some(FindMatch {
            span: 0..0,
            captures: vec![Capture::Position(0)]
        })
    );
    let mut session = Session::new();
    assert!(matches!(
        session.execute_multi(b"return string.find('a','(a')"),
        Err(AppError::Runtime(_))
    ));
    assert_eq!(
        session
            .execute_multi(b"return string.find('z','()')")
            .unwrap(),
        [Value::Number(1.0), Value::Number(0.0), Value::Number(1.0)]
    );
}

// ANCHOR: capture_real_owned_bytes
pub fn actual_vm_returns_owned_binary_capture_values() {
    let values = {
        let mut session = Session::new();
        let values = session
            .execute_multi(b"return string.find(string.char(65,0,255,66),'(A.*B)')")
            .unwrap();
        session
            .execute_multi(b"collectgarbage('collect');return string.find('zzz','(z+)')")
            .unwrap();
        values
    }; // Drop the Session and its temporary subject, keeping returned Values.
    assert_eq!(
        values,
        [
            Value::Number(1.0),
            Value::Number(4.0),
            Value::String(ByteString::new(b"A\0\xffB".as_slice()))
        ]
    );
}
// ANCHOR_END: capture_real_owned_bytes

pub fn actual_capture_and_work_limits_return_errors_without_destroying_the_session() {
    assert!(capture_checkpoint(&vec![CaptureState::Position(0); 32]).is_ok());
    assert!(matches!(
        capture_checkpoint(&vec![CaptureState::Position(0); 33]),
        Err(FindError::Limit(_))
    ));
    let full = find_captures(b"", &b"()".repeat(32), 1, false)
        .unwrap()
        .unwrap();
    assert_eq!(full.captures.len(), 32);
    assert!(matches!(
        find_captures(b"", &b"()".repeat(33), 1, false),
        Err(FindError::Limit(_))
    ));
    assert!(matches!(
        find_captures_with_limits(
            b"aaaaab",
            b"(a*)aab",
            1,
            false,
            Limits {
                steps: 30,
                backtrack_depth: 128
            }
        ),
        Err(FindError::Limit(_))
    ));
    assert_eq!(
        find_captures_with_limits(
            b"aaaaab",
            b"(a*)aab",
            1,
            false,
            Limits {
                steps: 100,
                backtrack_depth: 128
            }
        )
        .unwrap(),
        Some(FindMatch {
            span: 0..6,
            captures: vec![Capture::Bytes(0..3)]
        })
    );
    let mut session = Session::new();
    session.execute_multi(b"saved=40").unwrap();
    assert!(matches!(session.execute_multi(
        b"return string.find(string.rep('a',30),'(a*a*a*a*a*)b')"),
        Err(AppError::Runtime(error)) if error.kind == VmErrorKind::Limit));
    assert_eq!(
        session
            .execute_multi(b"return saved+2,string.find('ab','(a)b')")
            .unwrap(),
        [
            Value::Number(42.0),
            Value::Number(1.0),
            Value::Number(2.0),
            Value::String(ByteString::new(b"a".as_slice()))
        ]
    );
}

pub fn actual_checkpoint_restore_matches_the_teaching_projection() {
    let checkpoint =
        capture_checkpoint(&[CaptureState::Open(1), CaptureState::Position(2)]).unwrap();
    let mut actual = vec![
        CaptureState::Closed(1..5),
        CaptureState::Position(9),
        CaptureState::Open(8),
    ];
    let mut projected = actual.clone();
    restore_captures(&mut actual, &checkpoint);
    solution::restore_slots(&mut projected, checkpoint.states());
    assert_eq!(actual, projected);
    assert_eq!(actual, checkpoint.states());
}
