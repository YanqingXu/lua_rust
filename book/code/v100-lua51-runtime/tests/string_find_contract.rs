//! Fixed Lua-5.1 find subset oracle. Optional external Lua is checked by tools/check_string_find_contract.py.
use lua_app::{execute_multi, AppError, Session};
use lua_core::Value;
use lua_vm::string_find::{find, find_with_limits, FindError, Limits};
use lua_vm::VmErrorKind;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CORPUS: &str = include_str!("fixtures/string-find/cases.tsv");
struct Case {
    id: String,
    expected_stdout: Vec<u8>,
    error: bool,
}

// ANCHOR: find_fixed_corpus
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
            let expected = match fields[1] {
                "error" => {
                    assert_eq!(&fields[2..], &["-", "-"]);
                    b"error|after".to_vec()
                }
                "ok" => {
                    let values: Vec<_> = fields[3].split(',').collect();
                    assert_eq!(fields[2].parse::<usize>().unwrap(), values.len());
                    for value in &values {
                        assert!(
                            *value == "nil"
                                || value
                                    .strip_prefix("n:")
                                    .is_some_and(|digits| !digits.is_empty()
                                        && digits.bytes().all(|byte| byte.is_ascii_digit()))
                        );
                    }
                    assert!(
                        (values.len() == 1 && values[0] == "nil")
                            || (values.len() == 2
                                && values.iter().all(|value| value.starts_with("n:")))
                    );
                    format!("ok|{}|{}|after", fields[2], values.join("|")).into_bytes()
                }
                other => panic!("invalid result: {other}"),
            };
            Case {
                id: fields[0].to_owned(),
                expected_stdout: expected,
                error: fields[1] == "error",
            }
        })
        .collect()
}
// ANCHOR_END: find_fixed_corpus

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
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/string-find/driver.lua");
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
fn fixed_find_corpus_preserves_seventy_five_arities_and_byte_coordinates() {
    let cases = cases();
    assert_eq!(cases.len(), 75);
    assert_eq!(cases.iter().filter(|case| case.error).count(), 8);
    for (id, expected) in [
        ("init-past-end-empty", b"ok|2|n:4|n:3|after".as_slice()),
        ("empty-subject-empty", b"ok|2|n:1|n:0|after".as_slice()),
        ("literal-not-found", b"ok|1|nil|after".as_slice()),
    ] {
        assert_eq!(
            cases
                .iter()
                .find(|case| case.id == id)
                .unwrap()
                .expected_stdout,
            expected
        );
    }
}

// ANCHOR: find_binary_contract
#[test]
fn real_binary_matches_seventy_five_fixed_find_byte_cases() {
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
// ANCHOR_END: find_binary_contract

fn runtime_error(source: &str, kind: VmErrorKind) -> String {
    match execute_multi(source.as_bytes()).unwrap_err() {
        AppError::Runtime(error) => {
            assert_eq!(error.kind, kind, "{source}");
            error.message
        }
        other => panic!("expected runtime error for {source}, got {other:?}"),
    }
}

// ANCHOR: find_error_timing_contract
#[test]
fn find_errors_keep_argument_categories_and_only_validate_reached_atoms() {
    for (source, argument, category) in [
        ("return string.find()", "#1", "value expected"),
        ("return string.find('abc',{})", "#2", "string expected"),
        ("return string.find('abc','a',{})", "#3", "number expected"),
    ] {
        let message = runtime_error(source, VmErrorKind::Type);
        assert!(
            message.contains(argument) && message.contains(category),
            "{source}: {message}"
        );
    }
    for (pattern, category) in [
        ("a[", "missing ']'"),
        ("a%", "ends with '%'"),
        ("[", "missing ']'"),
        ("[^", "missing ']'"),
        ("[]", "missing ']'"),
    ] {
        let message = runtime_error(
            &format!("return string.find('abc','{pattern}')"),
            VmErrorKind::Type,
        );
        assert!(
            message.contains("malformed pattern") && message.contains(category),
            "{pattern}: {message}"
        );
    }
    for pattern in ["z[", "z%"] {
        assert_eq!(
            execute_multi(format!("return string.find('abc','{pattern}')").as_bytes()).unwrap(),
            [Value::Nil]
        );
    }
}
// ANCHOR_END: find_error_timing_contract

// ANCHOR: find_plain_and_empty_contract
#[test]
fn plain_truthiness_init_empty_ranges_and_c_prefixes_remain_distinct() {
    for (source, expected) in [
        (
            "return string.find('a.b','.',1,0)",
            vec![Value::Number(2.0), Value::Number(2.0)],
        ),
        (
            "return string.find('a.b','.',1,false)",
            vec![Value::Number(1.0), Value::Number(1.0)],
        ),
        (
            "return string.find('abc','',99)",
            vec![Value::Number(4.0), Value::Number(3.0)],
        ),
        (
            "return string.find('','',99)",
            vec![Value::Number(1.0), Value::Number(0.0)],
        ),
        (
            "return string.find('ab','.'..string.char(0)..'x')",
            vec![Value::Number(1.0), Value::Number(1.0)],
        ),
        (
            "return string.find('a'..string.char(0)..'x',string.char(0)..'.')",
            vec![Value::Nil],
        ),
    ] {
        assert_eq!(
            execute_multi(source.as_bytes()).unwrap(),
            expected,
            "{source}"
        );
    }
}
// ANCHOR_END: find_plain_and_empty_contract

// Candidate policy, not cases falsely advertised as equal to official supported features.
// ANCHOR: find_partial_rejection_contract
#[test]
fn unsupported_constructs_are_explicit_partial_errors_in_the_candidate() {
    for pattern in ["a?", "a-", "%b()", "%f[%a]", "%1"] {
        assert!(
            matches!(
                find(b"abc", pattern.as_bytes(), 1, false),
                Err(FindError::Unsupported(_))
            ),
            "{pattern}"
        );
        let message = runtime_error(
            &format!("return string.find('abc','{pattern}')"),
            VmErrorKind::Type,
        );
        assert!(
            message.contains("unsupported pattern"),
            "{pattern}: {message}"
        );
    }
    assert_eq!(find(b"abc", b"z%f[", 1, false), Ok(None));
    assert_eq!(find(b"a?", b"a?", 1, true), Ok(Some(0..2)));
    assert_eq!(find(b"abc", b"(b)", 1, false), Ok(Some(1..2)));
    assert!(matches!(
        find(b"a", b"(a", 1, false),
        Err(FindError::Invalid(_))
    ));
}
// ANCHOR_END: find_partial_rejection_contract

// ANCHOR: find_limit_contract
#[test]
fn candidate_work_limits_are_errors_and_leave_the_session_usable() {
    assert_eq!(
        find_with_limits(
            b"abc",
            b"abc",
            1,
            true,
            Limits {
                steps: 0,
                backtrack_depth: 0
            }
        ),
        Ok(Some(0..3)),
        "the pattern budget is not a universal plain-search budget"
    );
    for limits in [
        Limits {
            steps: 2,
            backtrack_depth: 128,
        },
        Limits {
            steps: 100,
            backtrack_depth: 1,
        },
    ] {
        let (subject, pattern) = if limits.steps == 2 {
            (b"aaaa".as_slice(), b"a*a".as_slice())
        } else {
            (b"aaac".as_slice(), b"a*a*b".as_slice())
        };
        assert!(matches!(
            find_with_limits(subject, pattern, 1, false, limits),
            Err(FindError::Limit(_))
        ));
    }
    let mut session = Session::new();
    session.execute_multi(b"saved=40").unwrap();
    let error = session
        .execute_multi(b"return string.find(string.rep('a',1000),'a*a*a*a*b')")
        .unwrap_err();
    assert!(
        matches!(error, AppError::Runtime(error) if error.kind == VmErrorKind::Limit && error.message.contains("pattern matching limit"))
    );
    assert_eq!(
        session.execute_multi(b"return saved+2").unwrap(),
        [Value::Number(42.0)]
    );
}
// ANCHOR_END: find_limit_contract
