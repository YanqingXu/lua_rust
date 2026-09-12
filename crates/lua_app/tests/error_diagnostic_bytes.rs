use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new() -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "lua-error-diagnostic-bytes-{}-{nonce}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("isolated diagnostic directory should be created");
        Self(path)
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_lua(directory: &Path, args: &[&str]) -> Output {
    // A process boundary catches aborts as failures without losing the test
    // harness. Keep output as bytes: invalid UTF-8 is the contract under test.
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(args)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("isolated diagnostic CLI should start");
    let deadline = Instant::now() + Duration::from_secs(30);
    let timed_out = loop {
        if child
            .try_wait()
            .expect("child status should be readable")
            .is_some()
        {
            break false;
        }
        if Instant::now() >= deadline {
            child.kill().expect("timed-out child should stop");
            break true;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = child
        .wait_with_output()
        .expect("raw child output should be captured");
    assert!(!timed_out, "diagnostic CLI timed out: {output:?}");
    output
}

fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn assert_raw_error(output: &Output, minimum_ff_count: usize) {
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert_raw_bytes(&output.stderr, minimum_ff_count);
}

fn assert_raw_bytes(bytes: &[u8], minimum_ff_count: usize) {
    // The official CLI additionally repeats source names in its traceback.
    assert!(
        bytes.iter().filter(|&&byte| byte == 0xff).count() >= minimum_ff_count,
        "{bytes:?}"
    );
    for forbidden in [b"\xef\xbf\xbd".as_slice(), b"\\xff", b"\\xFF"] {
        assert!(
            !contains(bytes, forbidden),
            "diagnostic bytes changed: {bytes:?}"
        );
    }
}

#[test]
fn loaders_and_protected_errors_preserve_bytes_and_object_types() {
    let directory = ScratchDirectory::new();
    let output = run_lua(
        &directory.0,
        &[
            "-e",
            include_str!("../../../tests/characterization/error-diagnostic-bytes.lua"),
            "-e",
            "print('next script passed')",
        ],
    );
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert!(
        output.stdout == b"error diagnostic bytes passed\nnext script passed\n"
            || output.stdout == b"error diagnostic bytes passed\r\nnext script passed\r\n",
        "{output:?}"
    );
}

#[test]
fn cli_syntax_errors_write_raw_token_bytes_to_stderr() {
    let directory = ScratchDirectory::new();
    std::fs::write(directory.0.join("invalid-byte.lua"), b"\xffa = 1\n").unwrap();
    assert_raw_error(&run_lua(&directory.0, &["invalid-byte.lua"]), 1);
}

#[test]
fn require_diagnostics_preserve_raw_bytes_in_protected_calls_and_cli_loading() {
    // Rust already exposes module compilation diagnostics; preserve their
    // bytes here. Stock Lua also exposes the parser bytes, while pinned C++
    // discards the parser error and reports only module-loading failure.
    // This regression does not settle that open compatibility-policy conflict.
    let directory = ScratchDirectory::new();
    std::fs::write(
        directory.0.join("error_diagnostic_module.lua"),
        b"\xffa = 1\n",
    )
    .unwrap();
    let protected = run_lua(
        &directory.0,
        &[
            "-e",
            r#"
            package.path = './?.lua'
            local ok, err = pcall(require, 'error_diagnostic_module')
            assert(not ok and type(err) == 'string')
            io.write(err, '\n')
            "#,
            "-e",
            "print('next script passed')",
        ],
    );
    assert!(protected.status.success(), "{protected:?}");
    assert!(protected.stderr.is_empty(), "{protected:?}");
    assert_raw_bytes(&protected.stdout, 1);
    assert!(
        protected.stdout.ends_with(b"\nnext script passed\n")
            || protected.stdout.ends_with(b"\r\nnext script passed\r\n"),
        "{protected:?}"
    );
    let unprotected = run_lua(
        &directory.0,
        &[
            "-e",
            "package.path = './?.lua'",
            "-l",
            "error_diagnostic_module",
            "-e",
            "print('unreachable')",
        ],
    );
    assert_raw_error(&unprotected, 1);
}

#[test]
fn cli_runtime_errors_write_raw_object_source_and_field_bytes_to_stderr() {
    let directory = ScratchDirectory::new();
    for (source, count) in [
        ("error(string.char(255), 0)", 1),
        (
            r#"assert(loadstring("error(string.char(255))", "=cli-\255"))()"#,
            2,
        ),
        (r#"local t={}; t["\255"]()"#, 1),
        (r#"local t={}; return t["\255"].x"#, 1),
        (r#"local t={}; return t["\255"]+1"#, 1),
    ] {
        assert_raw_error(&run_lua(&directory.0, &["-e", source]), count);
    }
}

#[test]
fn cli_literal_ascii_escape_is_not_decoded_into_a_byte() {
    let directory = ScratchDirectory::new();
    let output = run_lua(&directory.0, &["-e", r#"error("\\xff", 0)"#]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(contains(&output.stderr, b"\\xff"), "{output:?}");
    assert!(!output.stderr.contains(&0xff), "{output:?}");
    assert!(!contains(&output.stderr, b"\xef\xbf\xbd"), "{output:?}");
}
