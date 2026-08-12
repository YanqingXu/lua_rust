use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

// The fixed lua_cpp oracle exposes this project-qualified value instead of the
// stock Lua 5.1 `_VERSION` string.
const CPP_ORACLE_VERSION: &str = "Lua 5.1 (C core prototype)";

fn run_lua(source: &str, stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(["-e", source])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lua_app should start");

    let mut child_stdin = child.stdin.take().expect("child stdin should be piped");
    child_stdin
        .write_all(stdin)
        .expect("test input should be written");
    drop(child_stdin);

    child
        .wait_with_output()
        .expect("lua_app should finish and expose its output")
}

fn temp_path(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    path.push(format!(
        "lua_rust_observable_io_{name}_{}_{}",
        std::process::id(),
        stamp
    ));
    path
}

fn lua_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .replace('"', "\\\"")
}

fn run_lua_file(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should run the script file")
}

fn host_text_bytes(bytes: &[u8]) -> Vec<u8> {
    if cfg!(windows) {
        let mut translated =
            Vec::with_capacity(bytes.len() + bytes.iter().filter(|&&b| b == b'\n').count());
        for &byte in bytes {
            if byte == b'\n' {
                translated.push(b'\r');
            }
            translated.push(byte);
        }
        translated
    } else {
        bytes.to_vec()
    }
}

fn run_lua_file_with_args(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should run the script file with arguments")
}

fn run_lua_stdin_script(stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lua_app should start a stdin script");

    let mut child_stdin = child.stdin.take().expect("child stdin should be piped");
    child_stdin
        .write_all(stdin)
        .expect("stdin script should be written");
    drop(child_stdin);

    child
        .wait_with_output()
        .expect("lua_app should finish the stdin script")
}

fn run_lua_with_args_and_stdin(args: &[&str], stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lua_app should start with command-line arguments");

    let mut child_stdin = child.stdin.take().expect("child stdin should be piped");
    child_stdin
        .write_all(stdin)
        .expect("command stdin should be written");
    drop(child_stdin);

    child
        .wait_with_output()
        .expect("lua_app should finish the command")
}

fn run_lua_with_cwd_and_stdin(cwd: &Path, stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lua_app should start its full REPL");

    let mut child_stdin = child.stdin.take().expect("child stdin should be piped");
    child_stdin
        .write_all(stdin)
        .expect("full REPL input should be written");
    drop(child_stdin);

    child
        .wait_with_output()
        .expect("lua_app should finish its full REPL")
}

#[test]
fn snapshot_trace_is_jsonl_with_real_instruction_and_register_evidence() {
    let script_path = temp_path("snapshot-trace.lua");
    let trace_path = temp_path("snapshot-trace.jsonl");
    std::fs::write(
        &script_path,
        b"local function one() return 1 end; local value=one(); print(value)",
    )
    .expect("trace script should be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg("--trace")
        .arg(&trace_path)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should emit an instruction trace");
    let trace = std::fs::read_to_string(&trace_path).expect("trace file should be readable");
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&trace_path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
    assert!(
        output.stdout.starts_with(b"[TRACE] Trace enabled"),
        "missing trace start diagnostic: {output:?}"
    );
    assert!(
        output
            .stdout
            .windows(b"\n1\n".len())
            .any(|window| window == b"\n1\n")
            || output
                .stdout
                .windows(b"\r\n1\r\n".len())
                .any(|window| window == b"\r\n1\r\n"),
        "script stdout should remain visible between trace diagnostics: {output:?}"
    );
    assert!(
        output
            .stdout
            .windows(b"[TRACE] Trace complete:".len())
            .any(|window| { window == b"[TRACE] Trace complete:" }),
        "missing trace completion diagnostic: {output:?}"
    );

    let events: Vec<serde_json::Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
        .collect();
    assert!(!events.is_empty());
    for (sequence, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], sequence as u64);
    }
    let instructions: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "instruction")
        .collect();
    assert!(!instructions.is_empty());
    for event in &instructions {
        assert!(event["pc"].is_number());
        assert!(event["op"].is_string());
        assert!(event["funcName"].is_string());
        assert!(event["callDepth"].is_number());
        assert!(event["stackTop"].is_number());
        assert!(event["registers"].is_array());
    }
    assert!(
        instructions.iter().any(|event| event["callDepth"] == 2),
        "nested Lua execution should share the trace stream"
    );
    assert!(events.iter().any(|event| event["kind"] == "call"));
    assert!(events.iter().any(|event| event["kind"] == "return"));
}

#[test]
fn diff_trace_is_jsonl_with_post_instruction_register_changes() {
    let script_path = temp_path("diff-trace.lua");
    let trace_path = temp_path("diff-trace.jsonl");
    std::fs::write(
        &script_path,
        b"local function values(...) return ... end; local a,b=values(10,20); print(a+b)",
    )
    .expect("trace script should be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg("--trace-diff")
        .arg(&trace_path)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should emit an instruction-diff trace");
    let trace = std::fs::read_to_string(&trace_path).expect("trace file should be readable");
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&trace_path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
    assert!(
        output.stdout.starts_with(b"[TRACE] Trace diff enabled"),
        "missing diff trace start diagnostic: {output:?}"
    );

    let events: Vec<serde_json::Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
        .collect();
    assert!(!events.is_empty());
    for (sequence, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], sequence as u64);
    }
    let instructions: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "instruction")
        .collect();
    assert!(!instructions.is_empty());
    for event in &instructions {
        assert!(event["stackTop"].is_number());
        assert!(event["changedRegisters"].is_array());
        assert!(event.get("registers").is_none());
    }
    assert!(
        instructions.iter().any(|event| event["changedRegisters"]
            .as_array()
            .is_some_and(|items| !items.is_empty())),
        "diff trace should include at least one changed register"
    );
    assert!(
        instructions.iter().any(|event| event["op"] == "RETURN"),
        "final RETURN diff must be flushed"
    );
    assert!(events.iter().any(|event| event["kind"] == "call"));
    assert!(events.iter().any(|event| event["kind"] == "return"));
}

#[test]
fn callback_trace_restores_owner_stack_top_after_lua_metamethod_return() {
    for trace_flag in ["--trace", "--trace-diff"] {
        let script_path = temp_path("callback-stack-top.lua");
        let trace_path = temp_path("callback-stack-top.jsonl");
        std::fs::write(
            &script_path,
            br#"
local value = setmetatable({}, {
    __add = function()
        return 7
    end,
})

local result = value + 1
assert(result == 7)
"#,
        )
        .expect("callback stack-top script should be writable");

        let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
            .arg(trace_flag)
            .arg(&trace_path)
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("lua_app should trace a suspended metamethod opcode");
        let trace = std::fs::read_to_string(&trace_path)
            .expect("callback stack-top trace should be readable");
        let _ = std::fs::remove_file(&script_path);
        let _ = std::fs::remove_file(&trace_path);

        assert!(output.status.success(), "lua_app failed: {output:?}");
        assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
        let events: Vec<serde_json::Value> = trace
            .lines()
            .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
            .collect();
        let child_return = events
            .iter()
            .find(|event| {
                event["kind"] == "instruction"
                    && event["op"] == "RETURN"
                    && event["funcName"]
                        .as_str()
                        .is_some_and(|name| name.ends_with(":3"))
            })
            .expect("metamethod RETURN must remain in the trace");
        let outer_add = events
            .iter()
            .find(|event| event["kind"] == "instruction" && event["op"] == "ADD")
            .expect("suspended outer ADD must complete after the callback");

        if trace_flag == "--trace-diff" {
            assert_eq!(child_return["stackTop"], 0);
            assert_eq!(outer_add["stackTop"], 4);
        } else {
            let child_entry = events
                .iter()
                .find(|event| {
                    event["kind"] == "instruction"
                        && event["op"] == "LOADK"
                        && event["funcName"] == child_return["funcName"]
                })
                .expect("metamethod entry must establish its Snapshot capacity");
            let outer_entry = events
                .iter()
                .find(|event| {
                    event["kind"] == "instruction" && event["funcName"] == outer_add["funcName"]
                })
                .expect("outer frame must establish its Snapshot capacity");
            assert_eq!(child_return["stackTop"], child_entry["stackTop"]);
            assert_eq!(outer_add["stackTop"], outer_entry["stackTop"]);
        }
    }
}

#[test]
fn protected_calls_reject_yield_without_ghost_trace_and_restore_coroutine_permission() {
    let script_path = temp_path("protected-yield-barrier.lua");
    let trace_path = temp_path("protected-yield-barrier.jsonl");
    std::fs::write(
        &script_path,
        br#"
local ordinary = coroutine.create(function()
    return coroutine.yield("ordinary-yield")
end)
local ordinary_ok, ordinary_value = coroutine.resume(ordinary)
print("ordinary", ordinary_ok, ordinary_value)

local child = coroutine.create(function()
    return coroutine.yield("child-yield")
end)
local handler_child = coroutine.create(function()
    return "handler-child"
end)
local protected = coroutine.create(function()
    local target_ok, target_error = pcall(function()
        local child_ok, child_value = coroutine.resume(child)
        assert(child_ok and child_value == "child-yield")
        return coroutine.yield("forbidden-target")
    end)
    print("target", target_ok, string.find(target_error, "cannot yield", 1, true) ~= nil)

    local handler_ok, handler_error = xpcall(function()
        error("target failure")
    end, function()
        local child_ok, child_value = coroutine.resume(handler_child)
        assert(child_ok and child_value == "handler-child")
        return coroutine.yield("forbidden-handler")
    end)
    print("handler", handler_ok, string.find(handler_error, "cannot yield", 1, true) ~= nil)

    local nested_ok, nested_value = pcall(function()
        local inner_ok, inner_value = pcall(function() return "inner" end)
        assert(inner_ok and inner_value == "inner")
        return "outer"
    end)
    print("nested", nested_ok, nested_value)
    return coroutine.yield("allowed-after")
end)

local first_ok, first_value = coroutine.resume(protected)
print("first", first_ok, first_value)
local second_ok, second_value = coroutine.resume(protected, "finished")
print("second", second_ok, second_value)
"#,
    )
    .expect("protected-yield script should be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg("--trace-diff")
        .arg(&trace_path)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should run the protected-yield trace script");
    let trace = std::fs::read_to_string(&trace_path)
        .expect("protected-yield trace file should be readable");
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&trace_path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
    let stdout = String::from_utf8(output.stdout).expect("script stdout should be UTF-8");
    for expected in [
        "ordinary\ttrue\tordinary-yield",
        "target\tfalse\ttrue",
        "handler\tfalse\ttrue",
        "nested\ttrue\touter",
        "first\ttrue\tallowed-after",
        "second\ttrue\tfinished",
    ] {
        assert!(
            stdout.lines().any(|line| line == expected),
            "missing {expected:?} in stdout:\n{stdout}"
        );
    }

    let events: Vec<serde_json::Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
        .collect();
    let yields: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "yield")
        .collect();
    assert_eq!(
        yields.len(),
        3,
        "only the ordinary, nested-child, and post-protected yields are legal: {yields:#?}"
    );
}

#[test]
fn trace_records_unhandled_error_value_and_category() {
    let script_path = temp_path("error-trace.lua");
    let trace_path = temp_path("error-trace.jsonl");
    std::fs::write(&script_path, b"error('trace failure')")
        .expect("error trace script should be writable");

    let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
        .arg("--trace-diff")
        .arg(&trace_path)
        .arg(&script_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("lua_app should emit an unhandled-error trace");
    let trace = std::fs::read_to_string(&trace_path).expect("trace file should be readable");
    let _ = std::fs::remove_file(&script_path);
    let _ = std::fs::remove_file(&trace_path);

    assert_eq!(output.status.code(), Some(1));
    let events: Vec<serde_json::Value> = trace
        .lines()
        .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
        .collect();
    for (sequence, event) in events.iter().enumerate() {
        assert_eq!(event["seq"], sequence as u64);
    }
    let error = events
        .iter()
        .find(|event| event["kind"] == "error")
        .expect("trace should include an unhandled error event");
    let error_value = error["errorValue"]
        .as_str()
        .expect("errorValue should preserve the displayed string error");
    assert!(error_value.ends_with(":1: trace failure"));
    assert_eq!(error["errorCategory"], "runtime");
    assert_eq!(error["callDepth"], 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains(error_value));
}

#[test]
fn trace_records_upvalue_open_and_close_after_their_owning_instruction() {
    for trace_flag in ["--trace", "--trace-diff"] {
        let script_path = temp_path("upvalue-lifecycle.lua");
        let trace_path = temp_path("upvalue-lifecycle.jsonl");
        std::fs::write(
            &script_path,
            b"local function make(value) local function read() return value end; return read end; assert(make(7)() == 7)",
        )
        .expect("upvalue lifecycle script should be writable");

        let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
            .arg(trace_flag)
            .arg(&trace_path)
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("lua_app should emit an Upvalue lifecycle trace");
        let trace =
            std::fs::read_to_string(&trace_path).expect("Upvalue trace file should be readable");
        let _ = std::fs::remove_file(&script_path);
        let _ = std::fs::remove_file(&trace_path);

        assert!(output.status.success(), "lua_app failed: {output:?}");
        assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
        let events: Vec<serde_json::Value> = trace
            .lines()
            .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
            .collect();
        for (sequence, event) in events.iter().enumerate() {
            assert_eq!(event["seq"], sequence as u64);
        }

        let open_index = events
            .iter()
            .position(|event| event["kind"] == "upvalue-open")
            .expect("trace should include an Upvalue open event");
        assert_eq!(events[open_index - 1]["kind"], "instruction");
        assert_eq!(events[open_index - 1]["op"], "CLOSURE");
        assert_eq!(events[open_index]["slot"], 0);
        assert_eq!(events[open_index]["name"], "value");

        let close_index = events
            .iter()
            .position(|event| event["kind"] == "upvalue-close")
            .expect("trace should include an Upvalue close event");
        if trace_flag == "--trace" {
            assert_eq!(events[close_index - 1]["kind"], "return");
        } else {
            assert_eq!(events[close_index - 1]["kind"], "instruction");
            assert_eq!(events[close_index - 1]["op"], "RETURN");
        }
        assert_eq!(events[close_index]["slot"], 0);
        assert_eq!(events[close_index]["name"], "value");
    }
}

#[test]
fn trace_state_generations_prevent_collected_coroutines_from_suppressing_new_upvalues() {
    for trace_flag in ["--trace", "--trace-diff"] {
        let script_path = temp_path("coroutine-state-generation.lua");
        let trace_path = temp_path("coroutine-state-generation.jsonl");
        std::fs::write(
            &script_path,
            br#"
local function create_suspended(value)
    local thread = coroutine.create(function()
        local captured = value
        local function read()
            return captured
        end
        coroutine.yield(read)
    end)
    assert(coroutine.resume(thread))
end

for value = 1, 6 do
    create_suspended(value)
    if value < 6 then
        collectgarbage("collect")
    end
end
"#,
        )
        .expect("coroutine state-generation script should be writable");

        let output = Command::new(env!("CARGO_BIN_EXE_lua_app"))
            .arg(trace_flag)
            .arg(&trace_path)
            .arg(&script_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("lua_app should trace collected and replacement coroutines");
        let trace = std::fs::read_to_string(&trace_path)
            .expect("coroutine state-generation trace should be readable");
        let _ = std::fs::remove_file(&script_path);
        let _ = std::fs::remove_file(&trace_path);

        assert!(output.status.success(), "lua_app failed: {output:?}");
        assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
        let events: Vec<serde_json::Value> = trace
            .lines()
            .map(|line| serde_json::from_str(line).expect("each trace line should be valid JSON"))
            .collect();
        let captured_opens = events
            .iter()
            .filter(|event| event["kind"] == "upvalue-open" && event["name"] == "captured")
            .count();
        let captured_closes = events
            .iter()
            .filter(|event| event["kind"] == "upvalue-close" && event["name"] == "captured")
            .count();

        assert_eq!(
            captured_opens, 6,
            "each new StateHandle generation must publish its own open event"
        );
        assert_eq!(
            captured_closes, 5,
            "each explicitly collected suspended coroutine must close its tracked upvalue"
        );
    }
}

#[test]
fn script_syntax_errors_use_the_cpp_cli_diagnostic_envelope() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/lua/basic/syntax_error.lua")
        .canonicalize()
        .expect("syntax-error fixture should exist");
    let output = run_lua_file(&path);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let executable = Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("lua_app executable has a basename")
        .to_string_lossy();
    let line_ending = if cfg!(windows) { "\r\n" } else { "\n" };
    let expected = format!(
        "{executable}: {}:7: unexpected symbol near '<eof>'{line_ending}",
        path.to_string_lossy()
    );
    assert_eq!(output.stderr, expected.as_bytes());
}

#[test]
fn missing_script_files_use_the_stable_cpp_cli_diagnostic() {
    let path = temp_path("missing-script.lua");
    assert!(!path.exists(), "missing-script fixture must not exist");

    let output = run_lua_file(&path);

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let executable = Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("lua_app executable has a basename")
        .to_string_lossy();
    let expected = format!(
        "{executable}: cannot open file: {}\n",
        path.to_string_lossy()
    );
    assert_eq!(output.stderr, host_text_bytes(expected.as_bytes()));
}

#[test]
fn stdin_hash_bang_preserves_stdin_source_name_and_line_numbers() {
    let output = run_lua_stdin_script(
        b"#!/usr/bin/env lua\r\nio.write(debug.getinfo(1).source, '|', debug.getinfo(1).currentline)",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, b"=stdin|2");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");

    let output = run_lua_stdin_script(b"#!/usr/bin/env lua\nreturn return");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let executable = Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("lua_app executable has a basename")
        .to_string_lossy();
    let expected = format!("{executable}: stdin:2: unexpected symbol near 'return'\n");
    assert_eq!(output.stderr, host_text_bytes(expected.as_bytes()));
}

#[test]
fn interactive_mode_reads_lua_configured_prompts_after_startup_actions() {
    let output = run_lua_with_args_and_stdin(&["-e", "_PROMPT=''; _PROMPT2=''", "-i"], b"=6*7\n");

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, host_text_bytes(b"42\n\n"));
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn interactive_mode_reports_errors_and_continues_with_repl_source_names() {
    let output = run_lua_with_args_and_stdin(
        &["-e", "_PROMPT=''; _PROMPT2=''", "-i"],
        b"return return\n=2\nerror('boom')\n=3\n",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, host_text_bytes(b"2\n3\n\n"));
    assert_eq!(
        output.stderr,
        host_text_bytes(b"stdin:1: unexpected symbol near 'return'\n(repl):1: boom\n")
    );
}

#[test]
fn interactive_mode_prints_managed_string_results_from_multiline_input() {
    let output = run_lua_with_args_and_stdin(
        &["-e", "_PROMPT=''; _PROMPT2=''", "-i"],
        b"a = [[b\nc\nd\ne]]\n=type(a)\n=a\n",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, host_text_bytes(b"string\nb\nc\nd\ne\n\n"));
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn no_argument_repl_has_banner_help_history_and_goodbye_contract() {
    let directory = temp_path("full-repl");
    std::fs::create_dir(&directory).expect("isolated REPL directory should be created");
    let output = run_lua_with_cwd_and_stdin(&directory, b".help\n.unknown\n=1+2\nquit\n");
    let history = std::fs::read(directory.join(".lua_history"))
        .expect("full REPL should persist submitted history");
    std::fs::remove_file(directory.join(".lua_history"))
        .expect("isolated history should be removable");
    std::fs::remove_dir(&directory).expect("isolated REPL directory should be removable");

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(
        output.stdout,
        host_text_bytes(
            concat!(
                "Lua 5.1.5  Copyright (C) 1994-2012 Lua.org, PUC-Rio\n",
                "> REPL commands:\n",
                "  .help                  show this help\n",
                "  .bytecode <expr|chunk> compile input and print bytecode\n",
                "  .ast <expr|chunk>      parse input and print AST\n",
                "  .gc [stats|collect|strategy|help] inspect or run the active GC\n",
                "  =expr                  evaluate expression and print results\n",
                "  exit, quit             leave the REPL\n",
                "> > 3\n",
                "> Goodbye!\n",
            )
            .as_bytes()
        )
    );
    assert_eq!(
        output.stderr,
        host_text_bytes(b"unknown REPL command: .unknown\n")
    );
    assert_eq!(history, host_text_bytes(b".help\n.unknown\n=1+2\nquit\n"));
}

#[test]
fn runtime_errors_use_the_display_chunk_name_and_cpp_cli_envelope() {
    let output = run_lua(
        "local chunk = assert(loadstring('missing()', '=loader')); chunk()",
        b"",
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let executable = Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("lua_app executable has a basename")
        .to_string_lossy();
    let line_ending = if cfg!(windows) { "\r\n" } else { "\n" };
    let expected = format!(
        "{executable}: loader:1: attempt to call global 'missing' (a nil value){line_ending}"
    );
    assert_eq!(output.stderr, expected.as_bytes());
}

#[test]
fn long_control_structures_use_the_cpp_codegen_error_envelope() {
    let path = temp_path("control_structure_too_long.lua");
    let mut source = String::with_capacity(280_020);
    source.push_str("if false then\n");
    for _ in 0..70_000 {
        source.push_str("a=1\n");
    }
    source.push_str("end\n");
    std::fs::write(&path, source).expect("large compiler fixture should be written");

    let output = run_lua_file(&path);
    std::fs::remove_file(&path).expect("large compiler fixture should be removed");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let executable = Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("lua_app executable has a basename")
        .to_string_lossy();
    let expected = format!("{executable}: loadstring: control structure too long\n");
    assert_eq!(output.stderr, host_text_bytes(expected.as_bytes()));
}

#[test]
fn invalid_lvalues_use_the_cpp_codegen_error_envelope() {
    let executable = Path::new(env!("CARGO_BIN_EXE_lua_app"))
        .file_name()
        .expect("lua_app executable has a basename")
        .to_string_lossy();
    let expected = format!("{executable}: loadstring: Expression is not a valid lvalue\n");
    for source in ["1 = 2", "return function() 1 = 2 end"] {
        let output = run_lua(source, b"");
        assert_eq!(output.status.code(), Some(1), "source: {source}");
        assert!(output.stdout.is_empty(), "source: {source}");
        assert_eq!(
            output.stderr,
            host_text_bytes(expected.as_bytes()),
            "source: {source}"
        );
    }
}

#[test]
fn standard_output_error_and_cpp_oracle_version_are_process_visible() {
    let output = run_lua(
        "io.write('stdout|', _VERSION); io.flush(); io.stderr:write('stderr'); io.stderr:flush()",
        b"",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(
        output.stdout,
        format!("stdout|{CPP_ORACLE_VERSION}").as_bytes(),
        "io.write and _VERSION must be observable on the child stdout pipe"
    );
    assert_eq!(
        output.stderr, b"stderr",
        "io.stderr:write must be observable on the child stderr pipe"
    );
}

#[test]
fn standard_input_reads_from_the_child_stdin_pipe() {
    let output = run_lua(
        "local first = io.read('*l'); local second = io.stdin:read('*l'); io.write(first, '|', second)",
        b"alpha\nbeta\n",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, b"alpha|beta");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn standard_input_and_output_preserve_arbitrary_lua_bytes() {
    let input = [0x00, 0x80, 0xff, b'\n'];
    let output = run_lua("local bytes = io.read(4); io.write(bytes)", &input);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, host_text_bytes(&input));
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn standard_text_streams_expand_every_lf_without_reencoding_bytes() {
    let output = run_lua(
        "io.write(string.char(0,128,255,13,10)); io.flush(); \
         io.stderr:write(string.char(0,128,255,10)); io.stderr:flush()",
        b"",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(
        output.stdout,
        host_text_bytes(&[0x00, 0x80, 0xff, b'\r', b'\n'])
    );
    assert_eq!(output.stderr, host_text_bytes(&[0x00, 0x80, 0xff, b'\n']));
}

#[test]
fn print_writes_lua_string_bytes_without_utf8_reencoding() {
    let output = run_lua("print(string.char(0, 128, 255))", b"");

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, host_text_bytes(&[0x00, 0x80, 0xff, b'\n']));
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn script_file_preserves_non_utf8_source_bytes() {
    let path = temp_path("non-utf8-source.lua");
    let mut source = b"print(string.byte(\"".to_vec();
    source.push(0xe1);
    source.extend_from_slice(b"\"))");
    std::fs::write(&path, source).expect("non-UTF-8 Lua source should be writable");

    let output = run_lua_file(&path);
    let _ = std::fs::remove_file(path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, host_text_bytes(b"225\n"));
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn regular_text_and_binary_files_use_distinct_host_newline_contracts() {
    let text_path = temp_path("text-mode-newlines.bin");
    let binary_path = temp_path("binary-mode-newlines.bin");
    let source = format!(
        "local text=assert(io.open(\"{}\",'w')); \
         assert(text:write(string.char(65,10,66,13,10,67))); assert(text:close()); \
         local binary=assert(io.open(\"{}\",'wb')); \
         assert(binary:write(string.char(65,10,66,13,10,67))); assert(binary:close())",
        lua_path(&text_path),
        lua_path(&binary_path),
    );
    let output = run_lua(&source, b"");
    let text = std::fs::read(&text_path).expect("text-mode output should exist");
    let binary = std::fs::read(&binary_path).expect("binary-mode output should exist");
    let _ = std::fs::remove_file(&text_path);
    let _ = std::fs::remove_file(&binary_path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(text, host_text_bytes(b"A\nB\r\nC"));
    assert_eq!(binary, b"A\nB\r\nC");
}

#[test]
fn script_argument_table_and_varargs_survive_runtime_publication_handoff() {
    let path = temp_path("published-args.lua");
    std::fs::write(
        &path,
        b"local first, second = ...; io.write(arg[0], '|', arg[1], '|', arg[2], '|', first, '|', second)",
    )
    .expect("argument fixture should be writable");

    let output = run_lua_file_with_args(&path, &["alpha", "beta"]);
    let _ = std::fs::remove_file(&path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    let expected = format!("{}|alpha|beta|alpha|beta", path.to_string_lossy());
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn xpcall_handler_failure_keeps_false_and_one_published_error_result() {
    let output = run_lua(
        "local ok, err = xpcall(\
             function() error('primary failure') end,\
             function() error('handler failure') end\
         ); \
         assert(ok == false); \
         assert(type(err) == 'string' and string.find(err, 'handler failure')); \
         io.write('protected-error-ok')",
        b"",
    );

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(output.stdout, b"protected-error-ok");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}

#[test]
fn command_line_rejects_legacy_pseudo_dump_files() {
    let path = temp_path("legacy-pseudo-dump.lua");
    std::fs::write(
        &path,
        b"\x1bLuaRustDump:999:696f2e777269746528276c65676163792729",
    )
    .expect("legacy pseudo-dump fixture should be writable");

    let output = run_lua_file(&path);
    let _ = std::fs::remove_file(path);

    assert!(
        !output.status.success(),
        "legacy pseudo-dump unexpectedly executed: {output:?}"
    );
    assert!(
        output.stdout.is_empty(),
        "legacy pseudo-dump source fallback unexpectedly ran: {output:?}"
    );
    assert!(
        !output.stderr.is_empty(),
        "syntax rejection should be visible on stderr: {output:?}"
    );
}

#[test]
fn input_and_output_redirection_preserve_the_standard_handles() {
    let input_path = temp_path("input.txt");
    let output_path = temp_path("output.txt");
    std::fs::write(&input_path, b"from-file\n")
        .expect("redirected input fixture should be written");

    let source = format!(
        "io.input(\"{input}\"); local file_value = io.read('*l'); \
         io.input(io.stdin); local stdin_value = io.read('*l'); \
         io.output(\"{output}\"); io.write(file_value); io.flush(); \
         io.stdout:write(stdin_value); io.stdout:flush(); \
         io.output(io.stdout); io.write('|tail')",
        input = lua_path(&input_path),
        output = lua_path(&output_path),
    );
    let output = run_lua(&source, b"from-stdin\n");
    let redirected = std::fs::read(&output_path).expect("redirected output should exist");
    let _ = std::fs::remove_file(&input_path);
    let _ = std::fs::remove_file(&output_path);

    assert!(output.status.success(), "lua_app failed: {output:?}");
    assert_eq!(redirected, b"from-file");
    assert_eq!(output.stdout, b"from-stdin|tail");
    assert!(output.stderr.is_empty(), "unexpected stderr: {output:?}");
}
