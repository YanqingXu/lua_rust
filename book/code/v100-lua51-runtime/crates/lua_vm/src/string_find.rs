//! A byte-oriented Lua 5.1 `find` subset; captures and other pattern APIs stay partial.
use std::fmt;
use std::ops::Range;

// ANCHOR: find_atom_and_error
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Atom<'pattern> {
    Literal(u8),
    Any,
    Class(u8),
    /// Complete bracket spelling, including `[` and `]`.
    Set(&'pattern [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindError {
    Invalid(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
}
// ANCHOR_END: find_atom_and_error

impl fmt::Display for FindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(formatter, "malformed pattern ({reason})"),
            Self::Unsupported(reason) => write!(formatter, "unsupported pattern ({reason})"),
            Self::Limit(reason) => write!(formatter, "pattern matching limit ({reason})"),
        }
    }
}

impl std::error::Error for FindError {}

// ANCHOR: find_limits
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub steps: usize,
    pub backtrack_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            steps: 100_000,
            backtrack_depth: 128,
        }
    }
}

struct Budget(Limits);

impl Budget {
    fn charge(&mut self, steps: usize) -> Result<(), FindError> {
        self.0.steps = self
            .0
            .steps
            .checked_sub(steps)
            .ok_or(FindError::Limit("step budget exhausted"))?;
        Ok(())
    }
}
// ANCHOR_END: find_limits

// ANCHOR: find_parse_atom
/// Parse only the atom reached by matching; do not eagerly validate a whole pattern.
pub fn parse_atom(pattern: &[u8], at: usize) -> Result<(Atom<'_>, usize), FindError> {
    let byte = *pattern
        .get(at)
        .filter(|byte| **byte != 0)
        .ok_or(FindError::Invalid("expected atom before pattern end"))?;
    match byte {
        b'.' => Ok((Atom::Any, at + 1)),
        b'%' => match pattern.get(at + 1).filter(|byte| **byte != 0) {
            None => Err(FindError::Invalid("ends with '%'")),
            Some(b'b') => Err(FindError::Unsupported("balanced %b")),
            Some(b'f') => Err(FindError::Unsupported("frontier %f")),
            Some(b'0'..=b'9') => Err(FindError::Unsupported("capture backreference")),
            Some(class) => Ok((Atom::Class(*class), at + 2)),
        },
        b'[' => {
            let end = set_end(pattern, at)?;
            Ok((Atom::Set(&pattern[at..end]), end))
        }
        b'(' | b')' => Err(FindError::Unsupported("capture")),
        _ => Ok((Atom::Literal(byte), at + 1)),
    }
}
// ANCHOR_END: find_parse_atom

// Lua classend treats an initial `]` as content; a later unescaped `]` closes it.
fn set_end(pattern: &[u8], start: usize) -> Result<usize, FindError> {
    let mut cursor = start + 1;
    if pattern.get(cursor) == Some(&b'^') {
        cursor += 1;
    }
    loop {
        let byte = *pattern
            .get(cursor)
            .filter(|byte| **byte != 0)
            .ok_or(FindError::Invalid("missing ']'"))?;
        cursor += 1;
        if byte == b'%' && pattern.get(cursor).is_some_and(|byte| *byte != 0) {
            cursor += 1;
        }
        if pattern.get(cursor) == Some(&b']') {
            return Ok(cursor + 1);
        }
    }
}

// ANCHOR: find_ascii_class
fn class_matches(byte: u8, class: u8) -> bool {
    let matched = match class.to_ascii_lowercase() {
        b'a' => byte.is_ascii_alphabetic(),
        b'c' => byte.is_ascii_control(),
        b'd' => byte.is_ascii_digit(),
        b'l' => byte.is_ascii_lowercase(),
        b'p' => byte.is_ascii_punctuation(),
        b's' => matches!(byte, b' ' | b'\t'..=b'\r'),
        b'u' => byte.is_ascii_uppercase(),
        b'w' => byte.is_ascii_alphanumeric(),
        b'x' => byte.is_ascii_hexdigit(),
        b'z' => byte == 0,
        _ => return byte == class, // Unknown escapes denote their own byte in 5.1.
    };
    if class.is_ascii_lowercase() {
        matched
    } else {
        !matched
    }
}
// ANCHOR_END: find_ascii_class

fn set_matches(byte: u8, set: &[u8]) -> bool {
    if set.first() != Some(&b'[') || set.last() != Some(&b']') {
        return false;
    }
    let inverted = set.get(1) == Some(&b'^');
    let mut cursor = if inverted { 2 } else { 1 };
    let closing = set.len() - 1;
    while cursor < closing {
        let matched = if set[cursor] == b'%' {
            cursor += 1;
            class_matches(byte, set[cursor])
        } else if set.get(cursor + 1) == Some(&b'-') && cursor + 2 < closing {
            cursor += 2;
            set[cursor - 2] <= byte && byte <= set[cursor]
        } else {
            set[cursor] == byte
        };
        if matched {
            return !inverted;
        }
        cursor += 1;
    }
    inverted
}

impl Atom<'_> {
    pub fn matches(self, byte: u8) -> bool {
        match self {
            Self::Literal(expected) => byte == expected,
            Self::Any => true,
            Self::Class(class) => class_matches(byte, class),
            Self::Set(set) => set_matches(byte, set),
        }
    }

    fn checked_match(self, byte: u8, budget: &mut Budget) -> Result<bool, FindError> {
        let cost = if let Self::Set(set) = self {
            set.len()
        } else {
            1
        };
        budget.charge(cost)?;
        Ok(self.matches(byte))
    }
}

// ANCHOR: find_retry_state
struct Retry {
    minimum: usize,
    next: usize,
    pattern: usize,
}

fn retry(stack: &mut Vec<Retry>) -> Option<(usize, usize)> {
    let frame = stack.last_mut()?;
    let candidate = (frame.next, frame.pattern);
    if frame.next == frame.minimum {
        stack.pop();
    } else {
        frame.next -= 1;
    }
    Some(candidate)
}
// ANCHOR_END: find_retry_state

fn match_at(
    subject: &[u8],
    pattern: &[u8],
    start: usize,
    budget: &mut Budget,
) -> Result<Option<usize>, FindError> {
    let (mut source, mut cursor) = (start, 0);
    let mut stack = Vec::new();
    loop {
        budget.charge(1)?;
        if cursor == pattern.len() {
            return Ok(Some(source));
        }
        if pattern[cursor] == b'$' && cursor + 1 == pattern.len() {
            if source == subject.len() {
                return Ok(Some(source));
            }
        } else {
            let (atom, next) = parse_atom(pattern, cursor)?;
            let modifier = pattern.get(next).copied();
            if matches!(modifier, Some(b'?') | Some(b'-')) {
                return Err(FindError::Unsupported("optional or non-greedy repetition"));
            }
            if matches!(modifier, Some(b'*') | Some(b'+')) {
                let end = greedy_end(subject, source, atom, budget)?;
                let minimum = source + usize::from(modifier == Some(b'+'));
                if end >= minimum {
                    push_retry(&mut stack, minimum, end, next + 1, budget)?;
                    source = end;
                    cursor = next + 1;
                    continue;
                }
            } else if let Some(byte) = subject.get(source) {
                if atom.checked_match(*byte, budget)? {
                    source += 1;
                    cursor = next;
                    continue;
                }
            }
        }
        let Some((next_source, next_pattern)) = retry(&mut stack) else {
            return Ok(None);
        };
        source = next_source;
        cursor = next_pattern;
    }
}

// ANCHOR: find_greedy_candidates
fn greedy_end(
    subject: &[u8],
    start: usize,
    atom: Atom<'_>,
    budget: &mut Budget,
) -> Result<usize, FindError> {
    let mut end = start;
    while let Some(byte) = subject.get(end) {
        if !atom.checked_match(*byte, budget)? {
            break;
        }
        end += 1;
    }
    Ok(end)
}

fn push_retry(
    stack: &mut Vec<Retry>,
    minimum: usize,
    end: usize,
    pattern: usize,
    budget: &Budget,
) -> Result<(), FindError> {
    if end > minimum {
        if stack.len() >= budget.0.backtrack_depth {
            return Err(FindError::Limit("backtrack depth exhausted"));
        }
        stack.push(Retry {
            minimum,
            next: end - 1,
            pattern,
        });
    }
    Ok(())
}
// ANCHOR_END: find_greedy_candidates

// ANCHOR: find_init_and_branch
fn initial_offset(length: usize, init: i64) -> usize {
    if init > 0 {
        usize::try_from(init - 1).unwrap_or(length).min(length)
    } else if init == 0 {
        0
    } else {
        length.saturating_sub(usize::try_from(init.unsigned_abs()).unwrap_or(usize::MAX))
    }
}

fn c_prefix(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())]
}

fn plain_find(subject: &[u8], pattern: &[u8], start: usize) -> Option<Range<usize>> {
    if pattern.is_empty() {
        return Some(start..start);
    }
    subject[start..]
        .windows(pattern.len())
        .position(|window| window == pattern)
        .map(|offset| start + offset..start + offset + pattern.len())
}
// ANCHOR_END: find_init_and_branch

// ANCHOR: find_entry
pub fn find(
    subject: &[u8],
    pattern: &[u8],
    init: i64,
    plain: bool,
) -> Result<Option<Range<usize>>, FindError> {
    find_with_limits(subject, pattern, init, plain, Limits::default())
}

pub fn find_with_limits(
    subject: &[u8],
    pattern: &[u8],
    init: i64,
    plain: bool,
    limits: Limits,
) -> Result<Option<Range<usize>>, FindError> {
    let start = initial_offset(subject.len(), init);
    let prefix = c_prefix(pattern);
    if plain || !prefix.iter().any(|byte| b"^$*+?.([%-".contains(byte)) {
        return Ok(plain_find(subject, pattern, start));
    }
    let anchored = prefix.first() == Some(&b'^');
    let pattern = if anchored { &prefix[1..] } else { prefix };
    let mut budget = Budget(limits);
    for candidate in start..=subject.len() {
        if let Some(end) = match_at(subject, pattern, candidate, &mut budget)? {
            return Ok(Some(candidate..end));
        }
        if anchored {
            break;
        }
    }
    Ok(None)
}
// ANCHOR_END: find_entry

#[cfg(test)]
mod tests {
    use super::{find, find_with_limits, parse_atom, Atom, FindError, Limits};

    #[test]
    fn plain_and_automatic_plain_keep_literal_nul_and_ff_bytes() {
        assert_eq!(find(b"a.b", b".", 1, true), Ok(Some(1..2)));
        assert_eq!(find(b"a.b", b".", 1, false), Ok(Some(0..1)));
        assert_eq!(find(b"a\0\xffb", b"\0\xff", 1, false), Ok(Some(1..3)));
        assert_eq!(find(b"a\0x", b"\0.", 1, false), Ok(None));
        assert_eq!(find(b"ab", b".\0ignored[", 1, false), Ok(Some(0..1)));
    }

    #[test]
    fn init_clamps_to_both_ends_and_empty_matches_keep_half_open_range() {
        for init in [0, 1, -99, i64::MIN] {
            assert_eq!(find(b"abc", b"", init, false), Ok(Some(0..0)));
        }
        for init in [4, 99, i64::MAX] {
            assert_eq!(find(b"abc", b"", init, false), Ok(Some(3..3)));
        }
        assert_eq!(find(b"abc", b"c", -1, false), Ok(Some(2..3)));
        assert_eq!(find(b"abc", b"a", 99, false), Ok(None));
    }

    #[test]
    fn greedy_repetition_retries_a_shorter_prefix_before_rejecting_suffix() {
        assert_eq!(find(b"aaab", b"a*ab", 1, false), Ok(Some(0..4)));
        assert_eq!(find(b"aaab", b"a+ab", 1, false), Ok(Some(0..4)));
        assert_eq!(find(b"b", b"a*b", 1, false), Ok(Some(0..1)));
        assert_eq!(find(b"b", b"a+b", 1, false), Ok(None));
        assert_eq!(find(b"12foo34", b"%d+%a+%d+", 1, false), Ok(Some(0..7)));
    }

    #[test]
    fn reached_syntax_errors_do_not_hide_an_earlier_atom_mismatch() {
        assert_eq!(find(b"abc", b"z[", 1, false), Ok(None));
        assert!(matches!(
            find(b"abc", b"a[", 1, false),
            Err(FindError::Invalid(_))
        ));
        assert_eq!(find(b"abc", b"z%", 1, false), Ok(None));
        assert!(matches!(
            find(b"abc", b"a%", 1, false),
            Err(FindError::Invalid(_))
        ));
        assert!(matches!(
            find(b"", b"[", 1, false),
            Err(FindError::Invalid(_))
        ));
    }

    #[test]
    fn classes_sets_ranges_and_unknown_escapes_use_ascii_c_locale_rules() {
        assert_eq!(find(b"a_9", b"%w+", 1, false), Ok(Some(0..1)));
        assert_eq!(find(b"\x0bAZ9", b"%s%u+%d", 1, false), Ok(Some(0..4)));
        assert_eq!(find(b"a\0\xffb", b"%z.", 1, false), Ok(Some(1..3)));
        assert_eq!(find(b"abc89", b"[^a-z]+", 1, false), Ok(Some(3..5)));
        assert_eq!(find(b"f", b"[%f]", 1, false), Ok(Some(0..1)));
        assert_eq!(find(b"gG", b"%g%G", 1, false), Ok(Some(0..2)));
        assert_eq!(find(b"]-", b"[]%-]+", 1, false), Ok(Some(0..2)));
        assert!(Atom::Class(b'D').matches(255));
        assert!(!Atom::Class(b'a').matches(255));
    }

    #[test]
    fn anchors_are_relative_to_the_requested_start_and_the_whole_subject_end() {
        assert_eq!(find(b"abc", b"^b", 2, false), Ok(Some(1..2)));
        assert_eq!(find(b"abc", b"b$", 1, false), Ok(None));
        assert_eq!(find(b"abc", b"c$", 1, false), Ok(Some(2..3)));
        assert_eq!(find(b"abc", b"$", 99, false), Ok(Some(3..3)));
    }

    #[test]
    fn unsupported_constructs_are_separate_from_syntax_misses_and_limits() {
        for pattern in [b"a?".as_slice(), b"a-", b"(a)", b"%b()", b"%f[%a]", b"%1"] {
            assert!(
                matches!(
                    find(b"abc", pattern, 1, false),
                    Err(FindError::Unsupported(_))
                ),
                "{pattern:?}"
            );
        }
        assert_eq!(find(b"abc", b"z%f[", 1, false), Ok(None));
        assert_eq!(find(b")", b")", 1, false), Ok(Some(0..1)));
    }

    #[test]
    fn small_step_and_depth_budgets_fail_deterministically_without_recursion() {
        assert!(matches!(
            find_with_limits(
                b"aaaa",
                b"a*a",
                1,
                false,
                Limits {
                    steps: 2,
                    backtrack_depth: 128
                }
            ),
            Err(FindError::Limit("step budget exhausted"))
        ));
        assert!(matches!(
            find_with_limits(
                b"aaac",
                b"a*a*b",
                1,
                false,
                Limits {
                    steps: 100,
                    backtrack_depth: 1
                }
            ),
            Err(FindError::Limit("backtrack depth exhausted"))
        ));
        assert_eq!(
            find_with_limits(
                b"abc",
                b"abc",
                1,
                true,
                Limits {
                    steps: 0,
                    backtrack_depth: 0
                }
            ),
            Ok(Some(0..3))
        );
    }

    #[test]
    fn parser_returns_a_borrowed_atom_and_the_next_byte_position() {
        let pattern = b"x[^%d]+end";
        let (atom, next) = parse_atom(pattern, 1).unwrap();
        assert_eq!(atom, Atom::Set(&pattern[1..6]));
        assert_eq!(next, 6);
        assert!(atom.matches(b'A') && !atom.matches(b'3'));
        assert!(parse_atom(pattern, usize::MAX).is_err());
    }
}
