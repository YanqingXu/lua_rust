use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Inputs(PathBuf);

impl Inputs {
    fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temporary_root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary root should resolve to an absolute directory");
        let directory = temporary_root.join(format!(
            "lua-bytecode-diagnostic-{}-{nonce}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let directory = directory.canonicalize().unwrap();
        assert_eq!(directory.parent(), Some(temporary_root.as_path()));
        std::fs::write(directory.join("raw.lua"), b"'\xff'\n").unwrap();
        std::fs::write(directory.join("literal.lua"), b"'\\xff'\n").unwrap();
        std::fs::write(directory.join("valid.lua"), b"return 1\n").unwrap();
        Self(directory)
    }

    fn run(&self, args: &[&str]) -> Output {
        // Capture bytes across a process boundary: neither an abort nor an
        // invalid UTF-8 message may disappear inside the test harness.
        let mut child = Command::new(env!("CARGO_BIN_EXE_lua_bytecode"))
            .args(args)
            .current_dir(&self.0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("isolated bytecode CLI should start");
        let deadline = Instant::now() + Duration::from_secs(30);
        let timed_out = loop {
            if child.try_wait().unwrap().is_some() {
                break false;
            }
            if Instant::now() >= deadline {
                child.kill().expect("timed-out CLI should stop");
                break true;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let output = child.wait_with_output().unwrap();
        assert!(!timed_out, "bytecode CLI timed out: {output:?}");
        output
    }
}

impl Drop for Inputs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn assert_diagnostic(output: &Output, token_bytes: &[u8]) {
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        output.stdout.is_empty(),
        "partial output on failure: {output:?}"
    );
    let mut expected = b"[ERROR] Exception: unexpected symbol near ''".to_vec();
    expected.extend_from_slice(token_bytes);
    expected.extend_from_slice(b"''");
    expected.extend_from_slice(if cfg!(windows) { b"\r\n" } else { b"\n" });
    assert_eq!(output.stderr, expected, "{output:?}");
}

#[test]
fn text_cfg_and_diff_preserve_raw_bytes_and_literal_escape_identity() {
    // The pinned C++ tool distinguishes these two inputs. Their diagnostic
    // wording is identical, isolating raw FF transport from wording policy.
    let inputs = Inputs::new();
    for (filename, expected) in [
        ("raw.lua", b"\xff".as_slice()),
        ("literal.lua", b"\\xff".as_slice()),
    ] {
        for args in [
            vec![filename],
            vec![filename, "--full"],
            vec![filename, "--cfg"],
            vec![filename, "--cfg", "full"],
            vec![filename, "valid.lua", "--diff"],
            vec!["valid.lua", filename, "--diff", "--full"],
        ] {
            assert_diagnostic(&inputs.run(&args), expected);
        }
    }
}

#[test]
fn json_compilation_failures_keep_raw_text_stderr_and_exit_three() {
    // JSON is a Rust-only extension; pinned C++ rejects --format=json before
    // compiling. Rust's existing failure contract is text stderr, not JSON.
    let inputs = Inputs::new();
    for (filename, expected) in [
        ("raw.lua", b"\xff".as_slice()),
        ("literal.lua", b"\\xff".as_slice()),
    ] {
        assert_diagnostic(&inputs.run(&[filename, "--format=json"]), expected);
    }
}

#[test]
fn exception_envelope_stops_at_nul_like_the_pinned_cpp_tool() {
    let inputs = Inputs::new();
    std::fs::write(inputs.0.join("raw-nul.lua"), b"'raw\0suffix'\n").unwrap();
    let output = inputs.run(&["raw-nul.lua"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    // C++ prints e.what() up to NUL, then appends the envelope's newline.
    // The resulting diagnostic intentionally lacks the token's closing quote.
    let mut expected = b"[ERROR] Exception: unexpected symbol near ''raw".to_vec();
    expected.extend_from_slice(if cfg!(windows) { b"\r\n" } else { b"\n" });
    assert_eq!(output.stderr, expected, "{output:?}");
}
