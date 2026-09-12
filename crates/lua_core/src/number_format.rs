//! Lua-visible number formatting, separate from bytecode/serialization output.
//!
//! Lua 5.1 uses `%.14g`; the pinned C++ oracle uses `to_chars(general, 14)`.
//! General notation is selected from the exponent *after* decimal rounding.

/// Format a Lua number with the language's default 14 significant digits.
pub fn format_lua_number(number: f64) -> String {
    format_general(number, 14, false)
}

/// Format the significant-digit portion of a lowercase `%g` conversion.
///
/// `alternate` retains trailing zeroes and a decimal point, as `%#g` does.
/// A zero precision means one significant digit. Field width, plus/space
/// flags, and uppercase conversion belong to the caller.
pub fn format_general(number: f64, precision: usize, alternate: bool) -> String {
    if !number.is_finite() {
        return format_non_finite(number);
    }

    let precision = precision.max(1);
    // Rust's scientific formatter rounds the binary value once to the exact
    // requested decimal precision. Reconstructing from these digits avoids
    // double rounding and the log10-before-rounding notation boundary bug.
    let rounded = format!("{:.*e}", precision - 1, number.abs());
    let (coefficient, exponent) = rounded
        .split_once('e')
        .expect("finite scientific formatting has an exponent");
    let exponent: i32 = exponent.parse().expect("an f64 decimal exponent fits i32");
    let mut digits = coefficient.replace('.', "");
    if !alternate {
        while digits.len() > 1 && digits.ends_with('0') {
            digits.pop();
        }
    }

    let mut result = String::new();
    if number.is_sign_negative() {
        result.push('-');
    }
    if exponent < -4 || (exponent >= 0 && exponent as usize >= precision) {
        result.push(char::from(digits.as_bytes()[0]));
        if digits.len() > 1 || alternate {
            result.push('.');
            result.push_str(&digits[1..]);
        }
        result.push_str(&format!("e{exponent:+03}"));
    } else {
        let decimal_point = exponent + 1;
        if decimal_point <= 0 {
            result.push_str("0.");
            result.extend(std::iter::repeat_n('0', (-decimal_point) as usize));
            result.push_str(&digits);
        } else {
            let decimal_point = decimal_point as usize;
            if decimal_point < digits.len() {
                result.push_str(&digits[..decimal_point]);
                result.push('.');
                result.push_str(&digits[decimal_point..]);
            } else {
                result.push_str(&digits);
                result.extend(std::iter::repeat_n('0', decimal_point - digits.len()));
                if alternate {
                    result.push('.');
                }
            }
        }
    }
    result
}

fn format_non_finite(number: f64) -> String {
    let token = if number.is_infinite() {
        "inf"
    } else {
        nan_token(number)
    };
    if number.is_sign_negative() {
        format!("-{token}")
    } else {
        token.to_string()
    }
}

#[cfg(windows)]
fn nan_token(number: f64) -> &'static str {
    // MSVC's to_chars and UCRT %.g preserve these three NaN categories.
    // Inspect the bits without doing arithmetic that could quiet an sNaN.
    let bits = number.to_bits();
    if bits == 0xfff8_0000_0000_0000 {
        "nan(ind)"
    } else if bits & 0x0008_0000_0000_0000 == 0 {
        "nan(snan)"
    } else {
        "nan"
    }
}

#[cfg(not(windows))]
fn nan_token(_number: f64) -> &'static str {
    "nan"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_precision_and_notation_follow_rounded_exponents() {
        for (number, expected) in [
            (10.0 / 3.0, "3.3333333333333"),
            (1e-5, "1e-05"),
            (1e-4, "0.0001"),
            (1e13, "10000000000000"),
            (1e14, "1e+14"),
            (1e20, "1e+20"),
            (1e-308, "1e-308"),
            (f64::from_bits(1), "4.9406564584125e-324"),
            (99999999999999.5, "1e+14"),
            (0.0000999999999999995, "0.0001"),
            (f64::MAX, "1.7976931348623e+308"),
            (0.0, "0"),
            (-0.0, "-0"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
        ] {
            assert_eq!(format_lua_number(number), expected, "{number:?}");
        }
    }

    #[test]
    fn explicit_precision_and_alternate_form_preserve_significant_zeroes() {
        for (number, precision, alternate, expected) in [
            (999.5, 3, false, "1e+03"),
            (999.5, 3, true, "1.00e+03"),
            (0.0, 3, true, "0.00"),
            (-0.0, 3, true, "-0.00"),
            (12.0, 4, true, "12.00"),
            (120.0, 2, false, "1.2e+02"),
            (0.00009995, 3, false, "0.0001"),
            (1.5, 0, false, "2"),
            (2.5, 0, false, "2"),
            (1.0, 1, true, "1."),
        ] {
            assert_eq!(format_general(number, precision, alternate), expected);
        }
    }

    #[test]
    fn nan_payload_spelling_matches_the_platform_number_contract() {
        for (bits, windows, unix) in [
            (0x7ff8_0000_0000_0000, "nan", "nan"),
            (0xfff8_0000_0000_0000, "-nan(ind)", "-nan"),
            (0x7ff8_0000_0000_0001, "nan", "nan"),
            (0xfff8_0000_0000_0001, "-nan", "-nan"),
            (0x7ff0_0000_0000_0001, "nan(snan)", "nan"),
            (0xfff0_0000_0000_0001, "-nan(snan)", "-nan"),
        ] {
            assert_eq!(
                format_lua_number(f64::from_bits(bits)),
                if cfg!(windows) { windows } else { unix }
            );
        }
    }
}
