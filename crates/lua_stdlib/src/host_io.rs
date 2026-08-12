//! Host text-mode byte translation shared by the standard library and CLI.
//!
//! Rust's Windows standard streams and files are binary at the `Write` trait
//! boundary, while the fixed C++ oracle reaches them through the CRT's text
//! mode. Keep that platform conversion explicit and byte-oriented so Lua
//! strings never pass through UTF-8 re-encoding.

use std::io::{self, Write};

/// Writes bytes using the host C-runtime text-output contract.
///
/// On Windows every LF gains a preceding CR, including an LF that already
/// follows CR. On other platforms text and binary output are identical.
pub fn write_text(writer: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    {
        let mut start = 0;
        for (index, byte) in bytes.iter().copied().enumerate() {
            if byte == b'\n' {
                writer.write_all(&bytes[start..index])?;
                writer.write_all(b"\r\n")?;
                start = index + 1;
            }
        }
        writer.write_all(&bytes[start..])
    }

    #[cfg(not(windows))]
    {
        writer.write_all(bytes)
    }
}

/// Decodes bytes observed through a host C-runtime text input stream.
///
/// This is used for regular text files that are buffered logically before
/// being written back through [`write_text`]. Windows collapses CRLF and treats
/// Ctrl-Z as end-of-file; other platforms preserve every byte.
pub fn decode_text(bytes: &[u8]) -> Vec<u8> {
    #[cfg(windows)]
    {
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == 0x1a {
                break;
            }
            if bytes[index] == b'\r' && bytes.get(index + 1).copied() == Some(b'\n') {
                decoded.push(b'\n');
                index += 2;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
        decoded
    }

    #[cfg(not(windows))]
    {
        bytes.to_vec()
    }
}

/// Converts a logical text-buffer position to the host CRT's physical byte
/// offset. The returned value is suitable for Lua-visible `ftell` parity.
pub fn logical_to_physical_text_offset(bytes: &[u8], logical: usize) -> usize {
    let logical = logical.min(bytes.len());
    #[cfg(windows)]
    {
        logical
            + bytes[..logical]
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
    }

    #[cfg(not(windows))]
    {
        logical
    }
}

/// Converts a host CRT physical text offset back to a logical buffer cursor.
/// Offsets that land inside a translated CRLF pair select the following
/// logical byte boundary, matching a successful CRT seek/tell round trip.
pub fn physical_to_logical_text_offset(bytes: &[u8], physical: usize) -> usize {
    #[cfg(windows)]
    {
        let mut logical = 0;
        let mut current_physical = 0;
        while logical < bytes.len() && current_physical < physical {
            current_physical += if bytes[logical] == b'\n' { 2 } else { 1 };
            logical += 1;
        }
        logical
    }

    #[cfg(not(windows))]
    {
        physical.min(bytes.len())
    }
}

/// Whether text-mode output requires an explicit byte translation here.
pub const fn text_output_translates_lf() -> bool {
    cfg!(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_output_preserves_lua_bytes_and_uses_host_newlines() {
        let input = [0x00, 0x80, 0xff, b'\n', b'\r', b'\n'];
        let mut output = Vec::new();
        write_text(&mut output, &input).expect("Vec writes cannot fail");

        if cfg!(windows) {
            assert_eq!(
                output,
                [0x00, 0x80, 0xff, b'\r', b'\n', b'\r', b'\r', b'\n']
            );
        } else {
            assert_eq!(output, input);
        }
    }

    #[test]
    fn text_input_uses_the_host_crlf_and_ctrl_z_contract() {
        let input = [
            b'A', b'\r', b'\n', b'B', b'\r', b'\r', b'\n', b'C', 0x1a, b'D',
        ];
        let decoded = decode_text(&input);

        if cfg!(windows) {
            assert_eq!(decoded, b"A\nB\r\nC");
        } else {
            assert_eq!(decoded, input);
        }
    }

    #[test]
    fn text_offsets_map_logical_lf_bytes_to_host_positions() {
        let bytes = b"A\nB\r\nC";

        if cfg!(windows) {
            assert_eq!(logical_to_physical_text_offset(bytes, 0), 0);
            assert_eq!(logical_to_physical_text_offset(bytes, 2), 3);
            assert_eq!(logical_to_physical_text_offset(bytes, bytes.len()), 8);
            assert_eq!(physical_to_logical_text_offset(bytes, 3), 2);
            assert_eq!(physical_to_logical_text_offset(bytes, 8), bytes.len());
        } else {
            assert_eq!(logical_to_physical_text_offset(bytes, 2), 2);
            assert_eq!(
                logical_to_physical_text_offset(bytes, bytes.len()),
                bytes.len()
            );
            assert_eq!(physical_to_logical_text_offset(bytes, 3), 3);
        }
    }
}
