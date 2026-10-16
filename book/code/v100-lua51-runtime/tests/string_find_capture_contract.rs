//! Fixed capture oracle plus local error/ownership contracts, independent of external Lua installation.
use lua_app::{execute_multi, AppError, Session};
use lua_core::{ByteString, Value};
use lua_vm::string_find::{
    find, find_captures, find_captures_with_limits, Capture, FindError, FindMatch, Limits,
};
use lua_vm::VmErrorKind;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CORPUS: &str = include_str!("fixtures/string-find-captures/cases.tsv");
struct Case {
    id: String,
    expected_stdout: Vec<u8>,
    arity: Option<usize>,
}

fn valid_value(value: &str) -> bool {
    if value == "nil" {
        return true;
    }
    if let Some(digits) = value.strip_prefix("n:") {
        return !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    }
    let Some(text) = value.strip_prefix("s:") else {
        return false;
    };
    let Some((length, hex)) = text.split_once(':') else {
        return false;
    };
    length
        .parse::<usize>()
        .is_ok_and(|length| length == hex.len() / 2)
        && hex.len() % 2 == 0
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// ANCHOR: capture_fixed_corpus
fn cases() -> Vec<Case> {
    let mut lines = CORPUS.lines().filter(|line| !line.starts_with('#'));
    assert_eq!(lines.next(), Some("id\tresult\tarity\tvalues"));
    let mut ids = BTreeSet::new();
    lines
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 4);
            assert!(
                !fields[0].is_empty()
                    && fields[0]
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
            );
            assert!(ids.insert(fields[0]));
            let (expected_stdout, arity) = match fields[1] {
                "error" => {
                    assert_eq!(&fields[2..], &["-", "-"]);
                    (b"error|after".to_vec(), None)
                }
                "ok" => {
                    let arity = fields[2].parse::<usize>().unwrap();
                    let values: Vec<_> = fields[3].split(',').collect();
                    assert_eq!(arity, values.len());
                    assert!(values.iter().all(|value| valid_value(value)));
                    assert!((arity == 1 && values[0] == "nil") || (2..=34).contains(&arity));
                    (
                        format!("ok|{arity}|{}|after", values.join("|")).into_bytes(),
                        Some(arity),
                    )
                }
                other => panic!("invalid result class: {other}"),
            };
            Case {
                id: fields[0].to_owned(),
                expected_stdout,
                arity,
            }
        })
        .collect()
}
// ANCHOR_END: capture_fixed_corpus

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
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/string-find-captures/driver.lua");
    let child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(driver)
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
    drop(child.stdin.take());
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
fn capture_corpus_keeps_fifty_four_arities_lengths_and_binary_values() {
    let cases = cases();
    assert_eq!(cases.len(), 54);
    assert_eq!(cases.iter().filter(|case| case.arity.is_none()).count(), 7);
    assert_eq!(
        cases.iter().filter(|case| case.arity == Some(34)).count(),
        4
    );
    assert_eq!(
        cases
            .iter()
            .find(|case| case.id == "empty-string-capture")
            .unwrap()
            .expected_stdout,
        b"ok|3|n:1|n:0|s:0:|after"
    );
    assert_eq!(
        cases
            .iter()
            .find(|case| case.id == "binary-nul-high-byte")
            .unwrap()
            .expected_stdout,
        b"ok|3|n:2|n:3|s:2:00ff|after"
    );
}

// ANCHOR: capture_binary_contract
#[test]
fn real_binary_matches_fifty_four_fixed_capture_byte_cases() {
    for case in cases() {
        let actual = capture(&case).unwrap_or_else(|error| panic!("{}: {error}", case.id));
        assert_eq!(actual.status.code(), Some(0), "{} exit", case.id);
        assert_eq!(
            actual.stdout, case.expected_stdout,
            "{} raw stdout",
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
// ANCHOR_END: capture_binary_contract

fn error_message(source: &[u8], expected: VmErrorKind) -> String {
    match execute_multi(source).unwrap_err() {
        AppError::Runtime(error) => {
            assert_eq!(error.kind, expected);
            error.message
        }
        other => panic!("expected runtime error, got {other:?}"),
    }
}

// ANCHOR: capture_error_timing_contract
#[test]
fn capture_errors_keep_success_path_timing_and_distinct_categories() {
    for (subject, pattern, category) in [
        ("a", "(a", "unfinished capture"),
        ("a", "^a)", "invalid pattern capture"),
        ("a", "())", "invalid pattern capture"),
        ("a", "(a[", "missing ']'"),
    ] {
        let message = error_message(
            format!("return string.find('{subject}','{pattern}')").as_bytes(),
            VmErrorKind::Type,
        );
        assert!(
            message.contains("malformed pattern") && message.contains(category),
            "{pattern}: {message}"
        );
    }
    for (subject, pattern) in [("b", "(a"), ("b", "^a)"), ("ac", "(a$"), ("b", "(a[")] {
        assert_eq!(
            execute_multi(format!("return string.find('{subject}','{pattern}')").as_bytes())
                .unwrap(),
            [Value::Nil]
        );
    }
    for source in [
        "return string.find('',string.rep('()',33))",
        "return string.find(string.rep('a',33),string.rep('(a)',33))",
        "return string.find('a',string.rep('()',32)..'(a)')",
    ] {
        let message = error_message(source.as_bytes(), VmErrorKind::Limit);
        assert!(
            message.contains("too many captures") && message.contains("32"),
            "{message}"
        );
    }
    assert_eq!(
        execute_multi(b"return string.find('a','z'..string.rep('()',33))").unwrap(),
        [Value::Nil]
    );
}
// ANCHOR_END: capture_error_timing_contract

// ANCHOR: capture_retry_contract
#[test]
fn rollback_restores_closed_values_positions_and_new_slot_count() {
    for (pattern, expected) in [
        (b"(a*)ab".as_slice(), vec![Capture::Bytes(0..2)]),
        (b"a*(a)b".as_slice(), vec![Capture::Bytes(2..3)]),
        (
            b"((a*)a)b".as_slice(),
            vec![Capture::Bytes(0..3), Capture::Bytes(0..2)],
        ),
        (
            b"((a*)())ab".as_slice(),
            vec![
                Capture::Bytes(0..2),
                Capture::Bytes(0..2),
                Capture::Position(2),
            ],
        ),
    ] {
        let matched = find_captures(b"aaab", pattern, 1, false).unwrap().unwrap();
        assert_eq!(matched.span, 0..4);
        assert_eq!(matched.captures, expected, "{pattern:?}");
    }
    let matched = find_captures(b"xabc", b"()(a)b", 1, false)
        .unwrap()
        .unwrap();
    assert_eq!(matched.span, 1..3);
    assert_eq!(
        matched.captures,
        [Capture::Position(1), Capture::Bytes(1..2)]
    );
    assert_eq!(find_captures(b"x", b"(a)", 1, false), Ok(None));
    assert_eq!(
        find_captures(b"a", b"(a)", 1, false)
            .unwrap()
            .unwrap()
            .captures,
        [Capture::Bytes(0..1)]
    );
}
// ANCHOR_END: capture_retry_contract

// ANCHOR: capture_owned_contract
#[test]
fn owned_indices_vm_strings_and_legacy_range_projection_keep_their_boundaries() {
    let matched: FindMatch = {
        let subject = vec![0, 255, 10];
        find_captures(&subject, b"(.+)", 1, false).unwrap().unwrap()
    }; // FindMatch owns indices; it does not keep the discarded subject bytes alive.
    assert_eq!(matched.span, 0..3);
    assert_eq!(matched.captures, [Capture::Bytes(0..3)]);
    assert_eq!(find(b"aaab", b"(a*)ab", 1, false), Ok(Some(0..4)));
    assert!(matches!(
        find(b"a", b"(a", 1, false),
        Err(FindError::Invalid(_))
    ));
    let mut session = Session::new();
    let values = session
        .execute_multi(b"payload=string.char(0,255,10); return string.find(payload,'()(.*)()')")
        .unwrap();
    session
        .execute_multi(b"payload=nil; collectgarbage('collect'); return string.find('abc','(b)')")
        .unwrap();
    assert_eq!(
        values,
        [
            Value::Number(1.0),
            Value::Number(3.0),
            Value::Number(1.0),
            Value::String(ByteString::new([0, 255, 10])),
            Value::Number(4.0),
        ]
    );
}
// ANCHOR_END: capture_owned_contract

// These are explicit candidate policies, not extra claims of official equality.
// ANCHOR: capture_partial_and_limit_contract
#[test]
fn captures_do_not_enable_backreferences_or_erase_pattern_work_limits() {
    assert!(matches!(
        find_captures(b"aa", b"(a)%1", 1, false),
        Err(FindError::Unsupported(_))
    ));
    let message = error_message(b"return string.find('aa','(a)%1')", VmErrorKind::Type);
    assert!(message.contains("unsupported pattern") && message.contains("backreference"));
    let limited = find_captures_with_limits(
        b"aaab",
        b"(a*)ab",
        1,
        false,
        Limits {
            steps: 3,
            backtrack_depth: 128,
        },
    );
    assert!(matches!(limited, Err(FindError::Limit(_))));
    let message = error_message(
        b"return string.find(string.rep('a',1000),'(a*)a*a*a*b')",
        VmErrorKind::Limit,
    );
    assert!(message.contains("pattern matching limit") && !message.contains("too many captures"));
}
// ANCHOR_END: capture_partial_and_limit_contract
