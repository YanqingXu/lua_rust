//! v0.1: Lua values and a deliberately small single-literal evaluator.

use std::error::Error;
use std::fmt;

/// Lua strings are byte sequences, not necessarily UTF-8 text.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ByteString(Box<[u8]>);

impl ByteString {
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn escaped(&self) -> String {
        escape_bytes(self.as_bytes())
    }
}

impl fmt::Debug for ByteString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "ByteString(\"{}\")", self.escaped())
    }
}

/// The first complete runtime value model in the course.
// ANCHOR: value_model
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Nil,
    Boolean(bool),
    Number(f64),
    String(ByteString),
}
// ANCHOR_END: value_model

impl Value {
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Nil => "nil",
            Self::Boolean(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
        }
    }

    /// Lua considers only nil and false to be false.
    #[must_use]
    pub fn is_truthy(&self) -> bool {
        !matches!(self, Self::Nil | Self::Boolean(false))
    }

    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Nil => "nil".to_owned(),
            Self::Boolean(value) => value.to_string(),
            Self::Number(value) => format_number(*value),
            Self::String(value) => format!("\"{}\"", value.escaped()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralError {
    pub offset: usize,
    pub message: String,
}

impl LiteralError {
    fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}

impl fmt::Display for LiteralError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "byte {}: {}", self.offset, self.message)
    }
}

impl Error for LiteralError {}

/// Evaluate exactly one literal. v0.2 replaces this with a real lexer.
// ANCHOR: literal_eval
pub fn evaluate_literal(source: &[u8]) -> Result<Value, LiteralError> {
    let source = trim_ascii(source);
    if source.is_empty() {
        return Err(LiteralError::new(0, "expected one Lua literal"));
    }

    match source {
        b"nil" => Ok(Value::Nil),
        b"true" => Ok(Value::Boolean(true)),
        b"false" => Ok(Value::Boolean(false)),
        [quote @ (b'\'' | b'"'), ..] => parse_short_string(source, *quote),
        _ => parse_number(source).map(Value::Number),
    }
}
// ANCHOR_END: literal_eval

fn parse_number(source: &[u8]) -> Result<f64, LiteralError> {
    let text = std::str::from_utf8(source)
        .map_err(|error| LiteralError::new(error.valid_up_to(), "number is not ASCII"))?;
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return u64::from_str_radix(hex, 16)
            .map(|value| value as f64)
            .map_err(|_| LiteralError::new(0, "invalid hexadecimal number"));
    }

    text.parse::<f64>()
        .map_err(|_| LiteralError::new(0, "invalid number or unsupported literal"))
}

fn parse_short_string(source: &[u8], quote: u8) -> Result<Value, LiteralError> {
    let mut output = Vec::new();
    let mut index = 1;
    while index < source.len() {
        let byte = source[index];
        if byte == quote {
            if index + 1 != source.len() {
                return Err(LiteralError::new(
                    index + 1,
                    "unexpected bytes after string",
                ));
            }
            return Ok(Value::String(ByteString::from_bytes(&output)));
        }
        if byte != b'\\' {
            if byte == b'\n' || byte == b'\r' {
                return Err(LiteralError::new(index, "newline in short string"));
            }
            output.push(byte);
            index += 1;
            continue;
        }

        index += 1;
        let escaped = *source
            .get(index)
            .ok_or_else(|| LiteralError::new(index, "unfinished escape sequence"))?;
        match escaped {
            b'a' => output.push(7),
            b'b' => output.push(8),
            b'f' => output.push(12),
            b'n' => output.push(b'\n'),
            b'r' => output.push(b'\r'),
            b't' => output.push(b'\t'),
            b'v' => output.push(11),
            b'\\' | b'\'' | b'"' => output.push(escaped),
            b'\n' => output.push(b'\n'),
            b'\r' => {
                if source.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
                output.push(b'\n');
            }
            b'0'..=b'9' => {
                let start = index;
                let mut value = 0_u16;
                let mut count = 0;
                while count < 3 {
                    let Some(digit @ b'0'..=b'9') = source.get(index).copied() else {
                        break;
                    };
                    value = value * 10 + u16::from(digit - b'0');
                    index += 1;
                    count += 1;
                }
                if value > u16::from(u8::MAX) {
                    return Err(LiteralError::new(start, "decimal escape exceeds 255"));
                }
                output.push(value as u8);
                continue;
            }
            _ => output.push(escaped),
        }
        index += 1;
    }

    Err(LiteralError::new(source.len(), "unterminated string"))
}

fn trim_ascii(mut source: &[u8]) -> &[u8] {
    while source.first().is_some_and(u8::is_ascii_whitespace) {
        source = &source[1..];
    }
    while source.last().is_some_and(u8::is_ascii_whitespace) {
        source = &source[..source.len() - 1];
    }
    source
}

fn escape_bytes(bytes: &[u8]) -> String {
    let mut output = String::new();
    for &byte in bytes {
        match byte {
            b'\\' => output.push_str("\\\\"),
            b'"' => output.push_str("\\\""),
            b'\n' => output.push_str("\\n"),
            b'\r' => output.push_str("\\r"),
            b'\t' => output.push_str("\\t"),
            0x20..=0x7e => output.push(char::from(byte)),
            _ => output.push_str(&format!("\\x{byte:02x}")),
        }
    }
    output
}

fn format_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lua_truthiness_is_not_rust_truthiness() {
        assert!(!Value::Nil.is_truthy());
        assert!(!Value::Boolean(false).is_truthy());
        assert!(Value::Number(0.0).is_truthy());
        assert!(Value::String(ByteString::from_bytes(b"")).is_truthy());
    }

    #[test]
    fn arbitrary_string_bytes_are_preserved() {
        let value = evaluate_literal(b"\"a\\000\xff\"").unwrap();

        assert_eq!(
            value,
            Value::String(ByteString::from_bytes(&[b'a', 0, 0xff]))
        );
        assert_eq!(value.display(), "\"a\\x00\\xff\"");
    }

    #[test]
    fn hexadecimal_number_is_supported() {
        assert_eq!(evaluate_literal(b"0x2a"), Ok(Value::Number(42.0)));
    }
}
