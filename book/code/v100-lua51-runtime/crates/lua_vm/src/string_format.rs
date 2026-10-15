//! Bounded Lua 5.1 `string.format`: s/d/i/f/q/%%, without a C varargs boundary.
//!
//! Integer conversion deliberately covers the common signed 32-bit range.
//! Host C floating-point/locale edge cases and the other conversions remain partial.

use lua_core::Value;
use std::fmt;

// ANCHOR: format_spec
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Flags {
    pub left: bool,
    pub plus: bool,
    pub space: bool,
    pub alternate: bool,
    pub zero: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatSpec {
    pub flags: Flags,
    pub width: Option<u8>,
    pub precision: Option<u8>,
    pub conversion: u8,
}
// ANCHOR_END: format_spec

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatError {
    InvalidFormat(&'static str),
    UnsupportedConversion(u8),
    MissingArgument(usize),
    Type {
        argument: usize,
        expected: &'static str,
        actual: &'static str,
    },
    NumberOutOfRange(usize),
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat(reason) => write!(formatter, "invalid format ({reason})"),
            Self::UnsupportedConversion(byte) => {
                write!(
                    formatter,
                    "unsupported format option '%{}'",
                    char::from(*byte)
                )
            }
            Self::MissingArgument(argument) => {
                write!(
                    formatter,
                    "bad argument #{argument} to 'string.format' (no value)"
                )
            }
            Self::Type {
                argument,
                expected,
                actual,
            } => write!(
                formatter,
                "bad argument #{argument} to 'string.format' ({expected} expected, got {actual})"
            ),
            Self::NumberOutOfRange(argument) => write!(
                formatter,
                "bad argument #{argument} to 'string.format' (number outside supported range)"
            ),
        }
    }
}

impl std::error::Error for FormatError {}

// ANCHOR: format_parse_flags
fn parse_flags(format: &[u8], cursor: &mut usize) -> Result<Flags, FormatError> {
    let mut flags = Flags::default();
    let mut count = 0;
    while let Some(byte) = format.get(*cursor) {
        match byte {
            b'-' => flags.left = true,
            b'+' => flags.plus = true,
            b' ' => flags.space = true,
            b'#' => flags.alternate = true,
            b'0' => flags.zero = true,
            _ => break,
        }
        *cursor += 1;
        count += 1;
        // Lua 5.1 counts flag bytes; it does not reject every duplicate flag.
        if count > 5 {
            return Err(FormatError::InvalidFormat("repeated flags"));
        }
    }
    Ok(flags)
}
// ANCHOR_END: format_parse_flags

// ANCHOR: format_parse_digits
fn parse_digits(format: &[u8], cursor: &mut usize) -> Result<Option<u8>, FormatError> {
    let mut value = None;
    for _ in 0..2 {
        let Some(byte @ b'0'..=b'9') = format.get(*cursor) else {
            break;
        };
        value = Some(value.unwrap_or(0) * 10 + (byte - b'0'));
        *cursor += 1;
    }
    if format.get(*cursor).is_some_and(u8::is_ascii_digit) {
        return Err(FormatError::InvalidFormat("width or precision too long"));
    }
    Ok(value)
}
// ANCHOR_END: format_parse_digits

// ANCHOR: format_parse_spec
/// `start` points immediately after `%`; the returned cursor follows conversion.
/// Literal `%%` is handled by the dispatcher and does not enter this parser.
pub fn parse_spec(format: &[u8], start: usize) -> Result<(FormatSpec, usize), FormatError> {
    if start == 0 || format.get(start - 1) != Some(&b'%') {
        return Err(FormatError::InvalidFormat(
            "specifier cursor must follow '%'",
        ));
    }
    let mut cursor = start;
    let flags = parse_flags(format, &mut cursor)?;
    let width = parse_digits(format, &mut cursor)?;
    let precision = if format.get(cursor) == Some(&b'.') {
        cursor += 1;
        Some(parse_digits(format, &mut cursor)?.unwrap_or(0))
    } else {
        None
    };
    let conversion = *format
        .get(cursor)
        .ok_or(FormatError::InvalidFormat("ends with '%'"))?;
    if !matches!(conversion, b's' | b'd' | b'i' | b'f' | b'q') {
        return Err(FormatError::UnsupportedConversion(conversion));
    }
    Ok((
        FormatSpec {
            flags,
            width,
            precision,
            conversion,
        },
        cursor + 1,
    ))
}
// ANCHOR_END: format_parse_spec

fn type_error(argument: usize, expected: &'static str, value: &Value) -> FormatError {
    FormatError::Type {
        argument,
        expected,
        actual: value.type_name(),
    }
}

fn number(value: &Value, argument: usize) -> Result<f64, FormatError> {
    let number = match value {
        Value::Number(number) => *number,
        Value::String(bytes) => {
            // luaO_str2d receives a C string, unlike the length-aware %s fast path.
            let bytes = bytes
                .as_bytes()
                .split(|byte| *byte == 0)
                .next()
                .unwrap_or_default();
            let text = std::str::from_utf8(bytes.trim_ascii())
                .map_err(|_| type_error(argument, "number", value))?;
            if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                f64::from(
                    u32::from_str_radix(hex, 16)
                        .map_err(|_| type_error(argument, "number", value))?,
                )
            } else {
                text.parse::<f64>()
                    .map_err(|_| type_error(argument, "number", value))?
            }
        }
        _ => return Err(type_error(argument, "number", value)),
    };
    if !number.is_finite() {
        return Err(FormatError::NumberOutOfRange(argument));
    }
    Ok(number)
}

fn string_bytes(value: &Value, argument: usize) -> Result<Vec<u8>, FormatError> {
    match value {
        Value::String(bytes) => Ok(bytes.as_bytes().to_vec()),
        Value::Number(number) if number.is_finite() => Ok(number_string(*number).into_bytes()),
        Value::Number(_) => Err(FormatError::NumberOutOfRange(argument)),
        _ => Err(type_error(argument, "string", value)),
    }
}

// Lua's number-to-string path uses %.14g, rather than Rust's shortest display.
fn number_string(number: f64) -> String {
    let scientific = format!("{number:.13e}");
    let (mantissa, exponent_text) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let exponent = exponent_text.parse::<i32>().unwrap_or(0);
    if !(-4..14).contains(&exponent) {
        let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
        return format!("{mantissa}e{exponent:+03}");
    }
    let precision = usize::try_from(13 - exponent).unwrap_or(0);
    let fixed = format!("{number:.precision$}");
    if fixed.contains('.') {
        fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        fixed
    }
}

fn padded(spec: FormatSpec, sign: Option<u8>, digits: &[u8], zero: bool) -> Vec<u8> {
    let length = digits.len() + usize::from(sign.is_some());
    let padding = usize::from(spec.width.unwrap_or(0)).saturating_sub(length);
    let mut output = Vec::with_capacity(length + padding);
    if !spec.flags.left && !zero {
        output.resize(padding, b' ');
    }
    output.extend(sign);
    if !spec.flags.left && zero {
        output.resize(output.len() + padding, b'0');
    }
    output.extend_from_slice(digits);
    if spec.flags.left {
        output.resize(output.len() + padding, b' ');
    }
    output
}

fn sign(flags: Flags, negative: bool) -> Option<u8> {
    if negative {
        Some(b'-')
    } else if flags.plus {
        Some(b'+')
    } else if flags.space {
        Some(b' ')
    } else {
        None
    }
}

// ANCHOR: format_render_numbers
fn render_number(spec: FormatSpec, value: &Value, argument: usize) -> Result<Vec<u8>, FormatError> {
    let number = number(value, argument)?;
    if spec.conversion == b'f' {
        let precision = usize::from(spec.precision.unwrap_or(6));
        let mut digits = format!("{:.*}", precision, number.abs()).into_bytes();
        if precision == 0 && spec.flags.alternate {
            digits.push(b'.');
        }
        return Ok(padded(
            spec,
            sign(spec.flags, number.is_sign_negative()),
            &digits,
            spec.flags.zero,
        ));
    }
    let truncated = number.trunc();
    if truncated < f64::from(i32::MIN) || truncated > f64::from(i32::MAX) {
        return Err(FormatError::NumberOutOfRange(argument));
    }
    let integer = truncated as i32;
    let mut digits = integer.unsigned_abs().to_string().into_bytes();
    if spec.precision == Some(0) && integer == 0 {
        digits.clear();
    }
    let zeros = usize::from(spec.precision.unwrap_or(0)).saturating_sub(digits.len());
    if zeros > 0 {
        let mut prefix = vec![b'0'; zeros];
        prefix.extend(digits);
        digits = prefix;
    }
    Ok(padded(
        spec,
        sign(spec.flags, integer < 0),
        &digits,
        spec.flags.zero && spec.precision.is_none(),
    ))
}
// ANCHOR_END: format_render_numbers

// ANCHOR: format_render_string
fn render_string(spec: FormatSpec, value: &Value, argument: usize) -> Result<Vec<u8>, FormatError> {
    let bytes = string_bytes(value, argument)?;
    if spec.precision.is_none() && bytes.len() >= 100 {
        return Ok(bytes); // Lua 5.1 addvalue fast path bypasses width and C NUL.
    }
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let end = length.min(usize::from(spec.precision.unwrap_or(99)));
    Ok(padded(spec, None, &bytes[..end], false))
}
// ANCHOR_END: format_render_string

// ANCHOR: format_render_quote
fn render_quote(value: &Value, argument: usize) -> Result<Vec<u8>, FormatError> {
    let mut output = vec![b'"'];
    for byte in string_bytes(value, argument)? {
        match byte {
            b'"' | b'\\' | b'\n' => {
                output.push(b'\\');
                output.push(byte);
            }
            b'\r' => output.extend_from_slice(b"\\r"),
            0 => output.extend_from_slice(b"\\000"),
            _ => output.push(byte),
        }
    }
    output.push(b'"');
    Ok(output)
}
// ANCHOR_END: format_render_quote

// ANCHOR: format_dispatch
/// `arguments` excludes the format string; diagnostics still use Lua argument numbers.
pub fn format(format: &[u8], arguments: &[Value]) -> Result<Vec<u8>, FormatError> {
    let mut cursor = 0;
    let mut argument = 0;
    let mut output = Vec::new();
    while let Some(byte) = format.get(cursor) {
        if *byte != b'%' {
            output.push(*byte);
            cursor += 1;
        } else if format.get(cursor + 1) == Some(&b'%') {
            output.push(b'%');
            cursor += 2;
        } else {
            let number = argument + 2; // format is argument #1.
            let value = arguments
                .get(argument)
                .ok_or(FormatError::MissingArgument(number))?;
            let (spec, next) = parse_spec(format, cursor + 1)?;
            let item = match spec.conversion {
                b's' => render_string(spec, value, number)?,
                b'q' => render_quote(value, number)?,
                _ => render_number(spec, value, number)?,
            };
            output.extend(item);
            argument += 1;
            cursor = next;
        }
    }
    Ok(output)
}
// ANCHOR_END: format_dispatch

// ANCHOR: format_arguments
/// Check argument #1 through the same length-aware number/string boundary.
pub fn format_arguments(arguments: &[Value]) -> Result<Vec<u8>, FormatError> {
    let value = arguments.first().ok_or(FormatError::MissingArgument(1))?;
    let bytes = string_bytes(value, 1)?;
    format(&bytes, &arguments[1..])
}
// ANCHOR_END: format_arguments

#[cfg(test)]
mod tests {
    use super::{format, format_arguments, parse_spec, Flags, FormatError, FormatSpec};
    use lua_core::{ByteString, Value};

    fn bytes(value: &[u8]) -> Value {
        Value::String(ByteString::new(value))
    }

    #[test]
    fn spec_cursor_and_empty_precision_are_explicit() {
        let (spec, next) = parse_spec(b"[%+-08.3d]", 2).unwrap();
        assert_eq!(
            spec,
            FormatSpec {
                flags: Flags {
                    plus: true,
                    left: true,
                    zero: true,
                    ..Flags::default()
                },
                width: Some(8),
                precision: Some(3),
                conversion: b'd'
            }
        );
        assert_eq!(next, 9);
        assert_eq!(parse_spec(b"%.f", 1).unwrap().0.precision, Some(0));
        assert!(parse_spec(b"%++d", 1).is_ok());
    }

    #[test]
    fn malformed_specs_return_errors_without_panicking_or_reading_outside_input() {
        for input in [
            b"%100d".as_slice(),
            b"%.100f",
            b"%------s",
            b"%",
            b"%.",
            b"%*d",
            b"%x",
            b"%5%",
        ] {
            assert!(parse_spec(input, 1).is_err(), "{input:?}");
        }
        for start in [0, 4, usize::MAX] {
            assert!(parse_spec(b"%d", start).is_err());
        }
    }

    #[test]
    fn decimal_rendering_obeys_precision_sign_and_padding_precedence() {
        let n = Value::Number(7.9);
        assert_eq!(
            format(b"%+06d|% 6.3i|%-06d", &[n.clone(), n.clone(), n]).unwrap(),
            b"+00007|   007|7     "
        );
        assert_eq!(
            format(b"[%.0d][%+.0d]", &[Value::Number(0.0), Value::Number(0.0)]).unwrap(),
            b"[][+]"
        );
        assert_eq!(
            format(b"%d", &[Value::Number(f64::from(i32::MIN))]).unwrap(),
            b"-2147483648"
        );
        assert!(format(b"%d", &[Value::Number(f64::from(i32::MAX) + 1.0)]).is_err());
    }

    #[test]
    fn fixed_floats_have_six_default_digits_and_sign_aware_zero_padding() {
        assert_eq!(
            format(
                b"%f|%+08.2f|%#.0f|%-08.1f",
                &[
                    Value::Number(1.5),
                    Value::Number(-1.5),
                    Value::Number(2.0),
                    Value::Number(2.0)
                ]
            )
            .unwrap(),
            b"1.500000|-0001.50|2.|2.0     "
        );
        assert_eq!(format(b"%f", &[Value::Number(-0.0)]).unwrap(), b"-0.000000");
        assert!(format(b"%f", &[Value::Number(f64::INFINITY)]).is_err());
    }

    #[test]
    fn short_string_c_path_and_long_string_fast_path_have_different_nul_rules() {
        assert_eq!(format(b"[%5.2s]", &[bytes(b"A\0B")]).unwrap(), b"[    A]");
        let mut long = vec![b'x'; 100];
        long[2] = 0;
        assert_eq!(format(b"%99s", &[bytes(&long)]).unwrap(), long);
        assert_eq!(format(b"%.99s", &[bytes(&long)]).unwrap(), b"xx");
        assert_eq!(format(b"%.s", &[bytes(b"abc")]).unwrap(), b"");
    }

    #[test]
    fn quoting_preserves_lua51_escaped_physical_newline_and_binary_bytes() {
        let payload = b"a\n\r\0\xff\"\\";
        let quoted = format(b"%q", &[bytes(payload)]).unwrap();
        let source = [b"return ".as_slice(), &quoted].concat();
        let proto = lua_compiler::compile(&source).unwrap();
        assert_eq!(
            super::super::Vm::new().execute(&proto).unwrap(),
            bytes(payload)
        );
        assert_eq!(
            format(b"%99.1q", &[bytes(b"a\n\r\0\xff\"\\")]).unwrap(),
            b"\"a\\\n\\r\\000\xff\\\"\\\\\""
        );
        assert_eq!(format(b"%q", &[Value::Number(12.5)]).unwrap(), b"\"12.5\"");
    }

    #[test]
    fn percent_does_not_consume_arguments_and_errors_use_real_lua_positions() {
        assert_eq!(
            format(b"%%:%d:%%:%s", &[bytes(b"7"), Value::Number(2.5)]).unwrap(),
            b"%:7:%:2.5"
        );
        assert_eq!(
            format(b"%d%d", &[Value::Number(1.0)]),
            Err(FormatError::MissingArgument(3))
        );
        assert_eq!(format(b"%100d", &[]), Err(FormatError::MissingArgument(2)));
        assert!(matches!(
            format(b"%s", &[Value::Boolean(true)]),
            Err(FormatError::Type {
                argument: 2,
                expected: "string",
                ..
            })
        ));
        assert!(matches!(
            format(b"%d", &[Value::Nil]),
            Err(FormatError::Type {
                argument: 2,
                expected: "number",
                ..
            })
        ));
    }

    #[test]
    fn numeric_string_coercion_is_local_and_does_not_stringify_other_types() {
        for (number, expected) in [(1e14, b"1e+14".as_slice()), (1e-5, b"1e-05")] {
            assert_eq!(
                format_arguments(&[Value::Number(number)]).unwrap(),
                expected
            );
        }
        assert_eq!(format_arguments(&[]), Err(FormatError::MissingArgument(1)));
        assert!(matches!(
            format_arguments(&[Value::Boolean(true)]),
            Err(FormatError::Type {
                argument: 1,
                expected: "string",
                ..
            })
        ));
        assert_eq!(
            format(
                b"%d|%.1f|%d",
                &[bytes(b"0x10"), bytes(b" 1.5 "), bytes(b"12\0ignored")]
            )
            .unwrap(),
            b"16|1.5|12"
        );
        assert_eq!(
            format(b"%s|%q", &[Value::Number(1.25), Value::Number(42.0)]).unwrap(),
            b"1.25|\"42\""
        );
        assert!(matches!(
            format(b"%q", &[Value::Boolean(false)]),
            Err(FormatError::Type {
                expected: "string",
                ..
            })
        ));
        assert!(matches!(
            format(b"%f", &[bytes(b"not numeric")]),
            Err(FormatError::Type {
                expected: "number",
                ..
            })
        ));
    }
}
