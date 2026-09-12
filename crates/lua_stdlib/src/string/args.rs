use lua_core::value::Value;
use lua_vm::state::LuaState;

pub(super) fn argument_error(index: i32, function: &str, detail: &str) -> String {
    format!("bad argument #{index} to 'string.{function}' ({detail})")
}

pub(super) fn check_arity(state: &LuaState, count: i32, function: &str) -> Result<(), String> {
    if state.get_top() >= count {
        return Ok(());
    }
    let missing = if function == "format" {
        "format string"
    } else if count == 1 {
        "argument"
    } else {
        "arguments"
    };
    Err(format!("string.{function}: missing {missing}"))
}

pub(super) fn check_string(
    state: &LuaState,
    index: i32,
    function: &str,
) -> Result<Vec<u8>, String> {
    super::string_arg(state, index)
        .ok_or_else(|| argument_error(index, function, "string expected"))
}

pub(super) fn check_number(state: &LuaState, index: i32, function: &str) -> Result<f64, String> {
    let number = match state.at(index) {
        Some(Value::Number(number)) => Some(*number),
        Some(Value::String(reference)) => state
            .with_string_bytes(*reference, parse_number)
            .ok()
            .flatten(),
        _ => None,
    };
    number.ok_or_else(|| argument_error(index, function, "number expected"))
}

pub(super) fn check_integer(
    state: &LuaState,
    index: i32,
    function: &str,
    default: Option<i32>,
) -> Result<i32, String> {
    // The fixed C++ oracle defaults only absent arguments. Explicit nil is
    // present and must reach its normal numeric type check.
    if state.at(index).is_none()
        && let Some(default) = default
    {
        return Ok(default);
    }
    let number = check_number(state, index, function)?;
    checked_integer(number).map_err(|detail| argument_error(index, function, detail))
}

pub(super) fn checked_integer(number: f64) -> Result<i32, &'static str> {
    if !number.is_finite() {
        return Err("finite number expected");
    }
    let truncated = number.trunc();
    if truncated < f64::from(i32::MIN) || truncated > f64::from(i32::MAX) {
        return Err("number out of range");
    }
    Ok(truncated as i32)
}

fn parse_number(bytes: &[u8]) -> Option<f64> {
    let bytes = bytes.trim_ascii();
    let text = std::str::from_utf8(bytes).ok()?;
    let (sign, unsigned) = match bytes.first() {
        Some(b'-') => (-1.0, &bytes[1..]),
        Some(b'+') => (1.0, &bytes[1..]),
        _ => (1.0, bytes),
    };
    if unsigned.starts_with(b"0x") || unsigned.starts_with(b"0X") {
        return parse_hex(&unsigned[2..]).map(|number| sign * number);
    }
    let number = text.parse::<f64>().ok()?;
    // strtod reports ERANGE for an overflowing numeric string. Literal inf
    // and nan are still numeric inputs and reach the finite-value check.
    if number.is_infinite()
        && !unsigned.eq_ignore_ascii_case(b"inf")
        && !unsigned.eq_ignore_ascii_case(b"infinity")
    {
        return None;
    }
    if number == 0.0 {
        let significand = unsigned.split(|byte| matches!(byte, b'e' | b'E')).next()?;
        if significand.iter().any(|byte| matches!(byte, b'1'..=b'9')) {
            return None;
        }
    }
    Some(number)
}

fn parse_hex(bytes: &[u8]) -> Option<f64> {
    let mut number = 0.0;
    let mut fraction = None;
    let mut digits = 0;
    let mut index = 0;
    while let Some(byte) = bytes.get(index) {
        if *byte == b'.' && fraction.is_none() {
            fraction = Some(1.0);
        } else if byte.is_ascii_hexdigit() {
            let digit = (*byte as char).to_digit(16)?;
            if let Some(place) = &mut fraction {
                *place /= 16.0;
                number += f64::from(digit) * *place;
            } else {
                number = number * 16.0 + f64::from(digit);
            }
            digits += 1;
        } else {
            break;
        }
        index += 1;
    }
    if digits == 0 {
        return None;
    }
    if matches!(bytes.get(index), Some(b'p' | b'P')) {
        let exponent = std::str::from_utf8(&bytes[index + 1..])
            .ok()?
            .parse::<i32>()
            .ok()?;
        number = scale_power_two(number, exponent)?;
    } else if index != bytes.len() {
        return None;
    }
    number.is_finite().then_some(number)
}

fn scale_power_two(mut number: f64, mut exponent: i32) -> Option<f64> {
    if number == 0.0 {
        return Some(number);
    }
    // Scale in representable steps: exp2(1024) is infinite even when a small
    // hexadecimal significand makes the complete number representable.
    while exponent != 0 {
        let step = exponent.clamp(-512, 512);
        number *= f64::from(step).exp2();
        if !number.is_finite() || number == 0.0 {
            return None;
        }
        exponent -= step;
    }
    Some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_strings_preserve_whitespace_and_hex_conversion() {
        for (bytes, expected) in [
            (b" 2 \t".as_slice(), 2.0),
            (b"-0x2", -2.0),
            (b"+0X1.Ap1", 3.25),
            (b".5", 0.5),
            (b"0x1p-1074", f64::from_bits(1)),
            (b"0e99999999", 0.0),
            (b"0x0p99999999", 0.0),
            (b"0x.8p1024", 1023.0_f64.exp2()),
        ] {
            assert_eq!(parse_number(bytes), Some(expected));
        }
        for bytes in [
            b"0x".as_slice(),
            b"0x1p",
            b"0x1.2.3",
            b"2\0",
            b"\xff",
            b"1e999",
            b"1e-999",
            b"0x1p-9999",
        ] {
            assert_eq!(parse_number(bytes), None);
        }
    }

    #[test]
    fn integer_conversion_checks_finite_values_before_casting() {
        assert_eq!(checked_integer(f64::NAN), Err("finite number expected"));
        assert_eq!(
            checked_integer(f64::INFINITY),
            Err("finite number expected")
        );
        assert_eq!(checked_integer(2147483648.0), Err("number out of range"));
        assert_eq!(checked_integer(-2147483649.0), Err("number out of range"));
        assert_eq!(checked_integer(2147483647.75), Ok(i32::MAX));
        assert_eq!(checked_integer(-2147483648.75), Ok(i32::MIN));
        assert_eq!(checked_integer(-0.75), Ok(0));
    }
}
