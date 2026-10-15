mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "implement the bounded greedy retry-position TODO in starter.rs"]
fn retry_positions_keep_greedy_order_and_the_repetition_minimum() {
    contracts::retry_positions_keep_greedy_order_and_the_repetition_minimum(starter::next_position);
}
