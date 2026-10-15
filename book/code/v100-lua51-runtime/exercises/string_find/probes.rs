use crate::{faults, model::Retry, solution};
use lua_app::{execute_multi, AppError, Session};
use lua_core::{ByteString, Value};
use lua_vm::string_find::{find, find_with_limits, parse_atom, Atom, FindError, Limits};

pub fn plain_and_large_init_need_distinct_boundaries() {
    assert_eq!(faults::ignore_plain(b"abc", b".").unwrap(), Some(0..1));
    assert_eq!(find(b"abc", b".", 1, true).unwrap(), None);
    assert_eq!(faults::reject_large_init(b"abc", b"", 99), None);
    assert_eq!(find(b"abc", b"", 99, true).unwrap(), Some(3..3));
    assert_eq!(find(b"abc", b"", -99, true).unwrap(), Some(0..0));
    assert_eq!(find(b"abc", b"c", -1, true).unwrap(), Some(2..3));
}

pub fn literal_contains_cannot_match_byte_atoms_and_escaped_classes() {
    assert_eq!(faults::literal_contains(b"acb", b"a.b"), None);
    assert_eq!(find(b"acb", b"a.b", 1, false).unwrap(), Some(0..3));
    assert_eq!(find(b"abc7!", b"%d%p", 1, false).unwrap(), Some(3..5));
    assert_eq!(find(b"A.b", b"%.", 1, false).unwrap(), Some(1..2));
    assert_eq!(find(b"xB7", b"[A-C]%d", 1, false).unwrap(), Some(1..3));
}

// ANCHOR: string_find_real_retry
pub fn actual_atom_scanning_and_fixed_suffix_show_why_greedy_retries() {
    let subject = b"aaab";
    let pattern = b"a*ab";
    let (atom, after_atom) = parse_atom(pattern, 0).unwrap();
    assert!(matches!(atom, Atom::Literal(b'a')));
    assert_eq!(pattern[after_atom], b'*');
    let greedy_end = subject
        .iter()
        .take_while(|byte| atom.matches(**byte))
        .count();
    assert_eq!(greedy_end, 3);
    let mut broken = Retry::new(0..greedy_end, 0);
    let only = faults::greedy_only(&mut broken).unwrap();
    assert!(!subject[only..].starts_with(b"ab"));
    assert!(faults::greedy_only(&mut broken).is_none());
    let mut retry = Retry::new(0..greedy_end, 0);
    assert_eq!(solution::next_position(&mut retry), Some(3));
    let candidate = solution::next_position(&mut retry).unwrap();
    assert_eq!(candidate, 2);
    // This checks one fixed literal suffix; it is not another pattern matcher.
    assert!(subject[candidate..].starts_with(b"ab"));
    assert_eq!(find(subject, pattern, 1, false).unwrap(), Some(0..4));
    assert_eq!(
        execute_multi(b"return string.find('aaab','a*ab')").unwrap(),
        [Value::Number(1.0), Value::Number(4.0)]
    );
}
// ANCHOR_END: string_find_real_retry

pub fn plus_needs_one_byte_and_an_empty_match_is_still_some_range() {
    assert_eq!(find(b"ab", b"a*ab", 1, false).unwrap(), Some(0..2));
    assert_eq!(find(b"ab", b"a+ab", 1, false).unwrap(), None);
    assert_eq!(find(b"", b"a*", 1, false).unwrap(), Some(0..0));
    assert_eq!(find(b"", b"a+", 1, false).unwrap(), None);
    assert_eq!(
        execute_multi(b"return string.find('','a*')").unwrap(),
        [Value::Number(1.0), Value::Number(0.0)]
    );
    assert_eq!(
        execute_multi(b"return string.find('','a+')").unwrap(),
        [Value::Nil]
    );
}

// ANCHOR: string_find_real_lazy_error
pub fn an_unreached_malformed_suffix_is_not_an_eager_parse_error() {
    assert!(matches!(
        faults::parse_unreached_suffix(b"a%", 1),
        Err(FindError::Invalid(_))
    ));
    assert_eq!(find(b"b", b"a%", 1, false).unwrap(), None);
    assert_eq!(find(b"", b"a%", 1, false).unwrap(), None);
    assert!(matches!(
        find(b"a", b"a%", 1, false),
        Err(FindError::Invalid(_))
    ));
    assert_eq!(find(b"b", b"a[", 1, false).unwrap(), None);
    assert!(matches!(
        find(b"a", b"a[", 1, false),
        Err(FindError::Invalid(_))
    ));
}
// ANCHOR_END: string_find_real_lazy_error

pub fn actual_atom_parser_borrows_sets_and_matches_raw_bytes() {
    let pattern = b"[A-C]%d";
    let (atom, next) = parse_atom(pattern, 0).unwrap();
    assert_eq!(next, 5);
    let Atom::Set(borrowed) = atom else {
        panic!("expected the real borrowed set");
    };
    assert!(!borrowed.is_empty());
    assert!(atom.matches(b'B'));
    assert!(!atom.matches(b'Z'));
    let (digit, end) = parse_atom(pattern, next).unwrap();
    assert!(matches!(digit, Atom::Class(b'd')));
    assert_eq!(end, pattern.len());
    assert!(digit.matches(b'7'));
    assert!(!digit.matches(0xff));
    assert_eq!(find(b"a\0\xffb", b".%z.", 1, false).unwrap(), Some(0..3));
    assert_eq!(find(b"a\0\xffb", b"\0\xff", 1, true).unwrap(), Some(1..3));
}

pub fn a_limit_error_is_not_a_successful_no_match_result() {
    let result = find_with_limits(
        b"aaaaab",
        b"a*a*a*c",
        1,
        false,
        Limits {
            steps: 1,
            backtrack_depth: 1,
        },
    );
    assert!(matches!(result, Err(FindError::Limit(_))));
    assert_eq!(find(b"aaaaab", b"a*c", 1, false).unwrap(), None);
}

pub fn actual_vm_maps_half_open_ranges_and_recovers_after_a_find_error() {
    let mut session = Session::new();
    session.execute_multi(b"saved=40").unwrap();
    assert!(matches!(
        session.execute_multi(b"return string.find('a','a%')"),
        Err(AppError::Runtime(_))
    ));
    assert_eq!(
        session
            .execute_multi(b"return saved+2,string.find('abc','',99,true)")
            .unwrap(),
        [Value::Number(42.0), Value::Number(4.0), Value::Number(3.0)]
    );
    assert_eq!(
        session
            .execute_multi(b"return string.find('a.b','.',1,true)")
            .unwrap(),
        [Value::Number(2.0), Value::Number(2.0)]
    );
    assert_eq!(
        session
            .execute_multi(b"return string.sub('aaab',string.find('aaab','a*ab'))")
            .unwrap(),
        [Value::String(ByteString::new(b"aaab".as_slice()))]
    );
}
