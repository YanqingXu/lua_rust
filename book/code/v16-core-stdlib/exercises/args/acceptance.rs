mod contracts;
mod starter;

#[test]
#[ignore = "TODO 1: use Args number conversion and HostError indexing"]
fn number_coercion_and_errors_are_shared() {
    contracts::number_rules(starter::number);
}

#[test]
#[ignore = "TODO 2: use Args owned binary bytes and shared errors"]
fn bytes_remain_owned_and_binary_safe() {
    contracts::bytes_rules(starter::bytes);
}
