use crate::model::Retry;
use lua_vm::string_find::{find, parse_atom, FindError};
use std::ops::Range;

// ANCHOR: string_find_faults
pub fn ignore_plain(subject: &[u8], pattern: &[u8]) -> Result<Option<Range<usize>>, FindError> {
    find(subject, pattern, 1, false)
}

pub fn reject_large_init(subject: &[u8], pattern: &[u8], init: i64) -> Option<Range<usize>> {
    if init > subject.len() as i64 + 1 {
        return None;
    }
    find(subject, pattern, init, true).unwrap()
}

pub fn literal_contains(subject: &[u8], pattern: &[u8]) -> Option<Range<usize>> {
    if pattern.is_empty() {
        return Some(0..0);
    }
    subject
        .windows(pattern.len())
        .position(|window| window == pattern)
        .map(|start| start..start + pattern.len())
}

pub fn greedy_only(retry: &mut Retry) -> Option<usize> {
    retry.next.take()
}

pub fn parse_unreached_suffix(pattern: &[u8], offset: usize) -> Result<(), FindError> {
    parse_atom(pattern, offset).map(|_| ())
}
// ANCHOR_END: string_find_faults
