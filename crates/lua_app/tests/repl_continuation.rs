//! Piped REPL contracts verified against the pinned C++ CLI and stock Lua 5.1.5.
//! Quiet bare-name continuation follows the project CLI: comments, field names,
//! and subsequent incomplete lines do not inherit stock Lua's broader behavior.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
const QUIET_ARGS: &[&str] = &["-e", "_PROMPT='' _PROMPT2=''", "-i"];

struct ScratchDirectory(PathBuf);

impl ScratchDirectory {
    fn new() -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .expect("temporary directory should exist");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should follow the Unix epoch")
            .as_nanos();
        let path = root.join(format!(
            "lua-repl-continuation-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("isolated REPL directory should be created");
        Self(path)
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        // This absolute, uniquely created child of the temporary directory is
        // owned by this test, including any Full REPL .lua_history file.
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct ReapOnDrop(Child);

impl Drop for ReapOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run_repl(args: &[&str], input: &[u8]) -> Output {
    let scratch = ScratchDirectory::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lua_app"));
    command
        .args(args)
        .current_dir(&scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Inherited startup scripts must not alter this process contract. This is
    // isolation, not a claim that LUA_INIT support has been tested here.
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("LUA_INIT")
        {
            command.env_remove(name);
        }
    }
    let mut child = ReapOnDrop(command.spawn().expect("REPL should start"));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut stdin = child.0.stdin.take().expect("stdin should be piped");
    let mut stdout = child.0.stdout.take().expect("stdout should be piped");
    let mut stderr = child.0.stderr.take().expect("stderr should be piped");
    let input = input.to_vec();
    let writer = thread::spawn(move || stdin.write_all(&input));
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let (status, timed_out) = loop {
        if let Some(status) = child.0.try_wait().expect("child status should be readable") {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.0.kill().expect("timed-out REPL should stop");
            break (
                child.0.wait().expect("timed-out REPL should be reaped"),
                true,
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    let output = Output {
        status,
        stdout: stdout_reader
            .join()
            .expect("stdout reader should finish")
            .expect("stdout should be captured"),
        stderr: stderr_reader
            .join()
            .expect("stderr reader should finish")
            .expect("stderr should be captured"),
    };
    let write_result = writer.join().expect("stdin writer should finish");
    assert!(!timed_out, "REPL exceeded 30 seconds: {output:?}");
    write_result.expect("all input bytes should be delivered");
    output
}

fn host_text(text: &str) -> Vec<u8> {
    // Compare complete captured bytes; account only for the platform's actual
    // host text writer convention, without trimming or normalizing the output.
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

fn assert_repl(args: &[&str], input: &[u8], stdout: &str, stderr: &str) {
    let output = run_repl(args, input);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, host_text(stdout), "stdout: {output:?}");
    assert_eq!(output.stderr, host_text(stderr), "stderr: {output:?}");
}

#[test]
fn quiet_bare_name_with_spaces_and_tabs_continues_into_assignment() {
    assert_repl(
        QUIET_ARGS,
        b" \t_pending2 \t\n= 10\n=_pending2\n",
        "10\n\n",
        "",
    );
}

#[test]
fn quiet_comment_is_not_part_of_a_pending_assignment_name() {
    assert_repl(
        QUIET_ARGS,
        b"-- empty comment\ncommented -- trailing\n= 7\n=commented\n",
        "7\nnil\n\n",
        "stdin:1: unexpected symbol near 'commented'\n",
    );
}

#[test]
fn quiet_bare_name_exception_applies_only_to_the_first_line() {
    assert_repl(
        QUIET_ARGS,
        b"pending\n\n= 9\n=pending\n",
        "9\nnil\n\n",
        "stdin:1: unexpected symbol near 'pending'\n",
    );
}

#[test]
fn quiet_field_expression_does_not_continue_as_a_bare_name() {
    assert_repl(
        QUIET_ARGS,
        b"t={}\nt.x\n= 10\n=t.x\n",
        "10\nnil\n\n",
        "stdin:1: unexpected symbol near 't'\n",
    );
}

#[test]
fn full_repl_reports_bare_name_then_accepts_following_commands() {
    // Stock Lua's no-argument piped CLI is not this project's Full REPL mode.
    assert_repl(
        &[],
        b"pending\n=9\npending=4\n=pending\nquit\n",
        "Lua 5.1.5  Copyright (C) 1994-2012 Lua.org, PUC-Rio\n> > 9\n> > 4\n> Goodbye!\n",
        "stdin:1: unexpected symbol near 'pending'\n",
    );
}

#[test]
fn function_and_long_string_wait_for_their_actual_closing_tokens() {
    assert_repl(
        QUIET_ARGS,
        b"function f(x)\nreturn x,x+1\nend\n=f(3)\na=[[b\nc\nd\ne]]\n=a\n",
        "3\t4\nb\nc\nd\ne\n\n",
        "",
    );
}

#[test]
fn repeated_complete_syntax_errors_do_not_poison_later_commands() {
    assert_repl(
        QUIET_ARGS,
        b"return return\nprint(\"one\")\na={]\nprint(\"two\")\nreturn return\nprint(\"three\")\n",
        "one\ntwo\nthree\n\n",
        "stdin:1: unexpected symbol near 'return'\nstdin:1: unexpected symbol near ']'\nstdin:1: unexpected symbol near 'return'\n",
    );
}

#[test]
fn to_close_in_a_non_eof_syntax_error_is_terminal() {
    assert_repl(
        QUIET_ARGS,
        b"do else\nprint(\"after\")\n",
        "after\n\n",
        "stdin:1: Expected 'end' to close do block near 'else'\n",
    );
}

#[test]
fn codegen_failure_reports_once_then_accepts_the_next_command() {
    assert_repl(
        QUIET_ARGS,
        b"1=2\nprint(\"after\")\n",
        "after\n\n",
        "Expression is not a valid lvalue\n",
    );
}

#[test]
fn bare_break_codegen_error_does_not_enter_first_line_name_continuation() {
    assert_repl(
        QUIET_ARGS,
        b"break\nprint(\"after\")\n=10\n",
        "after\n10\n\n",
        "no loop to break\n",
    );
}

#[test]
fn batch_break_codegen_error_exits_without_executing_an_unpatched_jump() {
    let output = run_repl(&["-e", "break"], b"");
    let executable = std::path::Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("CLI executable should have a file name")
        .to_str()
        .expect("CLI executable name should be UTF-8");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert_eq!(output.stdout, b"", "{output:?}");
    assert_eq!(
        output.stderr,
        host_text(&format!("{executable}: loadstring: no loop to break\n")),
        "{output:?}"
    );
}

#[test]
fn protected_loadstring_break_returns_a_codegen_diagnostic_and_continues() {
    assert_repl(
        &[
            "-e",
            "local ok,f,e=pcall(loadstring,\"break\"); print(ok,f,e); print(\"after\")",
        ],
        b"",
        "true\tnil\tloadstring: no loop to break\nafter\n",
        "",
    );
}

#[test]
fn eof_abandons_incomplete_source_without_executing_it() {
    assert_repl(QUIET_ARGS, b"if true then\n", "\n", "");
}

macro_rules! runtime_marker_case {
    ($name:ident, $marker:literal) => {
        #[test]
        fn $name() {
            assert_repl(
                QUIET_ARGS,
                concat!(
                    "io.write(\"side;\"); error(\"",
                    $marker,
                    "\",0)\nprint(\"after\")\n"
                )
                .as_bytes(),
                "side;after\n\n",
                concat!($marker, "\n"),
            );
        }
    };
}

runtime_marker_case!(runtime_eof_marker_does_not_reexecute_side_effects, "<eof>");
runtime_marker_case!(runtime_to_close_marker_is_terminal, "to close");
runtime_marker_case!(
    runtime_unterminated_string_marker_is_terminal,
    "unterminated string"
);
runtime_marker_case!(
    runtime_unfinished_string_marker_is_terminal,
    "unfinished string"
);
runtime_marker_case!(
    runtime_unfinished_long_string_marker_is_terminal,
    "unfinished long string"
);
runtime_marker_case!(
    runtime_unfinished_long_comment_marker_is_terminal,
    "unfinished long comment"
);
