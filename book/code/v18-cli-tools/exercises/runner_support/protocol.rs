//! Shared literal protocol; each teaching target intentionally uses only part of it.
#![allow(dead_code)]

pub const FLOOD_BYTES: usize = 1024 * 1024;
pub const STDIN_READY: &[u8] = b"READY stdin\n";
pub const STDIN_ACK: &[u8] = b"ACK stdin\n";
pub const STDIN_EOF: &[u8] = b"EOF stdin\n";
pub const DUAL_READY: &[u8] = b"READY dual\n";
pub const DUAL_ACK: &[u8] = b"ACK dual\n";
pub const STALL_STDOUT: &[u8] = b"READY stall\nPARTIAL\0\xff\n";
pub const STALL_STDERR: &[u8] = b"PARTIAL stderr\0\xfe\n";
pub const EXIT_STDOUT: &[u8] = b"OUT\0\xff\n";
pub const EXIT_STDERR: &[u8] = b"ERR\0\xfe\n";
pub const VERSION: &[u8] = b"runner-fixture v1\n";

pub fn stdin_stdout(input: &[u8]) -> Vec<u8> {
    [STDIN_READY, STDIN_ACK, b"INPUT:", input].concat()
}

pub fn stdout_payload() -> Vec<u8> {
    (0..FLOOD_BYTES)
        .map(|index| ((index * 17 + 3) % 256) as u8)
        .collect()
}

pub fn stderr_payload() -> Vec<u8> {
    (0..FLOOD_BYTES)
        .map(|index| ((index * 31 + 11) % 256) as u8)
        .collect()
}

pub fn dual_stdout() -> Vec<u8> {
    [DUAL_READY, DUAL_ACK, &stdout_payload()].concat()
}

pub fn dual_stderr() -> Vec<u8> {
    stderr_payload()
}
