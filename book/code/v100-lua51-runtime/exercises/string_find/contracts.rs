use crate::model::{NextPosition, Retry};

// ANCHOR: string_find_contract
pub fn retry_positions_keep_greedy_order_and_the_repetition_minimum(next: NextPosition) {
    for (consumed, minimum, expected) in [
        (0..3, 0, vec![3, 2, 1, 0]),
        (0..3, 1, vec![3, 2, 1]),
        (0..0, 0, vec![0]),
        (0..0, 1, vec![]),
        (2..5, 0, vec![5, 4, 3, 2]),
        (2..5, 1, vec![5, 4, 3]),
    ] {
        let mut retry = Retry::new(consumed.clone(), minimum);
        let mut actual = Vec::new();
        // A bound turns a repeated-position bug into a test failure, not a hang.
        for _ in 0..=expected.len() {
            match next(&mut retry) {
                Some(position) => actual.push(position),
                None => break,
            }
        }
        assert_eq!(actual, expected);
        assert!(
            next(&mut retry).is_none(),
            "exhaustion must remain exhausted"
        );
        assert!(next(&mut retry).is_none());
        assert_eq!(retry.consumed, consumed, "borrowed bounds are not shifted");
        assert_eq!(retry.minimum, minimum);
    }
    let mut overflow = Retry::new(usize::MAX..usize::MAX, 1);
    assert!(next(&mut overflow).is_none());
    assert!(overflow.next.is_none());
}
// ANCHOR_END: string_find_contract
