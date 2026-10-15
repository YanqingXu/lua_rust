use std::ops::Range;

/// A projection of retry positions after a one-byte atom greedily consumed input.
pub struct Retry {
    pub consumed: Range<usize>,
    pub minimum: usize,
    pub next: Option<usize>,
}

impl Retry {
    pub fn new(consumed: Range<usize>, minimum: usize) -> Self {
        let next = Some(consumed.end);
        Self {
            consumed,
            minimum,
            next,
        }
    }
}

pub type NextPosition = fn(&mut Retry) -> Option<usize>;
