//! Fixed Lua-5.1 byte oracle; external Lua is optional and is checked by tools/check_string_format_contract.py.
use lua_app::{execute_multi, AppError};
use lua_vm::VmErrorKind;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CORPUS: &str = include_str!("fixtures/string-format/cases.tsv");

struct Case {
    id: String,
    expected_stdout: Vec<u8>,
    value: Option<Vec<u8>>,
}

fn decode_hex(field: &str) -> Vec<u8> {
    assert_eq!(field.len() % 2, 0, "odd hex");
    assert!(field
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    field
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

// ANCHOR: format_fixed_corpus
fn cases() -> Vec<Case> {
    let mut lines = CORPUS.lines().filter(|line| !line.starts_with('#'));
    assert_eq!(lines.next(), Some("id\tresult\tvalue_hex"));
    let mut ids = BTreeSet::new();
    lines
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 3);
            assert!(
                !fields[0].is_empty()
                    && fields[0]
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'-')
            );
            assert!(ids.insert(fields[0]));
            let (expected_stdout, value) = match fields[1] {
                "ok" => (
                    format!("ok|{}|after", fields[2]).into_bytes(),
                    Some(decode_hex(fields[2])),
                ),
                "error" => {
                    assert!(fields[2].is_empty());
                    (b"error|after".to_vec(), None)
                }
                other => panic!("invalid result class: {other}"),
            };
            Case {
                id: fields[0].to_owned(),
                expected_stdout,
                value,
            }
        })
        .collect()
}
// ANCHOR_END: format_fixed_corpus

struct ChildOwner(Option<Child>);

impl ChildOwner {
    fn stop(&mut self) -> Result<(), String> {
        let Some(child) = self.0.as_mut() else {
            return Ok(());
        };
        let killed = child.kill();
        child
            .wait()
            .map_err(|error| format!("wait after kill ({killed:?}): {error}"))?;
        self.0.take();
        Ok(())
    }
}

impl Drop for ChildOwner {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn drain(mut pipe: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn capture(case: &Case) -> Result<Output, String> {
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/string-format/driver.lua");
    let child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(script)
        .arg(&case.id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn: {error}"))?;
    let mut owner = ChildOwner(Some(child));
    let child = owner.0.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let output_reader = thread::spawn(move || drain(stdout));
    let error_reader = thread::spawn(move || drain(stderr));
    drop(child.stdin.take()); // No request payload; close the last writer for EOF.
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match owner.0.as_mut().unwrap().try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => break Err(format!("{} exceeded five-second deadline", case.id)),
            Err(error) => break Err(format!("try_wait: {error}")),
        }
    };
    let status = match status {
        Ok(status) => {
            owner.0.take();
            Ok(status)
        }
        Err(error) => match owner.stop() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; additionally {cleanup}")),
        },
    };
    let stdout = output_reader.join();
    let stderr = error_reader.join();
    Ok(Output {
        status: status?,
        stdout: stdout.map_err(|_| "stdout reader panicked")??,
        stderr: stderr.map_err(|_| "stderr reader panicked")??,
    })
}

#[test]
fn fixed_format_corpus_has_fifty_seven_raw_cases() {
    let cases = cases();
    assert_eq!(cases.len(), 57);
    for (id, expected) in [
        ("quote-binary", b"\"\\000\xff\\\n\\r\\\"\\\\\"".as_slice()),
        ("format-nul-then-integer", b"A\x007".as_slice()),
        ("number-string-exponents", b"1e+20:\"1e-05\"".as_slice()),
    ] {
        assert_eq!(
            cases
                .iter()
                .find(|case| case.id == id)
                .unwrap()
                .value
                .as_deref(),
            Some(expected)
        );
    }
    assert_eq!(cases.iter().filter(|case| case.value.is_none()).count(), 16);
}

// ANCHOR: format_binary_contract
#[test]
fn real_binary_matches_fifty_seven_fixed_format_byte_cases() {
    for case in cases() {
        let actual = capture(&case).unwrap_or_else(|error| panic!("{}: {error}", case.id));
        assert_eq!(actual.status.code(), Some(0), "{} exit", case.id);
        assert_eq!(
            actual.stdout, case.expected_stdout,
            "{} full stdout bytes",
            case.id
        );
        assert!(
            actual.stderr.is_empty(),
            "{} stderr: {:?}",
            case.id,
            actual.stderr
        );
    }
}
// ANCHOR_END: format_binary_contract

fn error_message(source: &str) -> String {
    match execute_multi(source.as_bytes()).unwrap_err() {
        AppError::Runtime(error) => {
            assert_eq!(error.kind, VmErrorKind::Type, "{source}");
            error.message
        }
        other => panic!("expected runtime Type error for {source}, got {other:?}"),
    }
}

// These local VM diagnostics are a separate contract from the Lua pcall byte output.
// ANCHOR: format_error_type_contract
#[test]
fn format_type_errors_keep_kind_and_argument_number() {
    for (source, argument, category) in [
        ("return string.format('%s',{})", "#2", "string expected"),
        ("return string.format('%s',false)", "#2", "string expected"),
        ("return string.format('%q',true)", "#2", "string expected"),
        ("return string.format('%d',{})", "#2", "number expected"),
        ("return string.format('%f',nil)", "#2", "number expected"),
        ("return string.format({})", "#1", "string expected"),
        ("return string.format(false)", "#1", "string expected"),
        ("return string.format(nil)", "#1", "string expected"),
    ] {
        let message = error_message(source);
        assert!(
            message.contains(argument) && message.contains(category),
            "{source}: {message}"
        );
    }
}
// ANCHOR_END: format_error_type_contract

// ANCHOR: format_missing_priority_contract
#[test]
fn missing_values_take_priority_before_format_parsing() {
    for (source, argument) in [
        ("return string.format()", "#1"),
        ("return string.format('%s')", "#2"),
        ("return string.format('%z')", "#2"),
        ("return string.format('%100d')", "#2"),
        ("return string.format('%%:%d:%s',7)", "#3"),
    ] {
        let message = error_message(source);
        assert!(
            message.contains(argument) && message.contains("no value"),
            "{source}: {message}"
        );
        assert!(!message.contains("invalid format") && !message.contains("unsupported format"));
    }
}
// ANCHOR_END: format_missing_priority_contract

// ANCHOR: format_invalid_spec_contract
#[test]
fn invalid_format_errors_do_not_become_argument_errors() {
    for (source, category) in [
        ("return string.format('%z',1)", "format option '%z'"),
        (
            "return string.format('%100d',1)",
            "width or precision too long",
        ),
        (
            "return string.format('%.100f',1)",
            "width or precision too long",
        ),
        ("return string.format('%------d',1)", "repeated flags"),
        ("return string.format('%',1)", "ends with '%'"),
    ] {
        let message = error_message(source);
        assert!(message.contains(category), "{source}: {message}");
        assert!(!message.contains("bad argument"), "{source}: {message}");
    }
}
// ANCHOR_END: format_invalid_spec_contract
