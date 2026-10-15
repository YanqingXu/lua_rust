mod contracts;
mod model;
mod starter;

#[test]
#[ignore = "TODO: encode a string/hex object with exact lowercase byte pairs"]
fn bytes_constants_preserve_exact_bytes_as_lower_hex() {
    contracts::exact_bytes(starter::encode_bytes);
}
