use crate::model::Retry;

// ANCHOR: string_find_solution
pub fn next_position(retry: &mut Retry) -> Option<usize> {
    let position = retry.next.take()?;
    let lower = retry.consumed.start.checked_add(retry.minimum)?;
    if position < lower || position > retry.consumed.end {
        return None;
    }
    retry.next = position
        .checked_sub(1)
        .filter(|previous| *previous >= lower);
    Some(position)
}
// ANCHOR_END: string_find_solution
