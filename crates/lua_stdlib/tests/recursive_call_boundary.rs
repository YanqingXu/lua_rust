#![forbid(unsafe_code)]

use lua_compiler::{codegen::CodeGenerator, parser::Parser};
use lua_core::value::Value;
use lua_vm::Runtime;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const STACK_BYTES_ENV: &str = "LUA_RUST_RECURSIVE_CHILD_STACK_BYTES";
const CHILD_TEST: &str = "recursive_host_thread_child";
const SOURCE: &str = include_str!("../../../tests/characterization/recursive-error-boundary.lua");

#[test]
fn recursive_execution_is_safe_on_multiple_host_thread_stacks() {
    let executable = std::env::current_exe().expect("test executable has a path");
    for stack_bytes in [1024 * 1024, 2 * 1024 * 1024, 8 * 1024 * 1024] {
        let mut child = Command::new(&executable)
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
            .env(STACK_BYTES_ENV, stack_bytes.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("isolated recursive host test should start");
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
            .expect("child output should be captured");
        assert!(
            !timed_out,
            "{stack_bytes}-byte host stack timed out: {output:?}"
        );
        assert!(
            output.status.success(),
            "{stack_bytes}-byte host stack: {output:?}"
        );
        assert!(
            output.stderr.is_empty(),
            "{stack_bytes}-byte host stack: {output:?}"
        );
        let stdout = String::from_utf8(output.stdout).expect("test output should be UTF-8");
        assert!(
            stdout.contains("recursive error boundary continued"),
            "{stack_bytes}-byte host stack: {stdout}"
        );
        assert!(
            stdout.contains("host thread completed"),
            "{stack_bytes}-byte host stack: {stdout}"
        );
    }
}

#[test]
fn recursive_host_thread_child() {
    let Ok(stack_bytes) = std::env::var(STACK_BYTES_ENV) else {
        // Only the driver opts into this child. A normal harness invocation
        // must never execute the regression on its own host stack.
        return;
    };
    let stack_bytes = stack_bytes
        .parse::<usize>()
        .expect("valid child stack size");
    std::thread::Builder::new()
        .name(format!("lua-recursion-{stack_bytes}"))
        .stack_size(stack_bytes)
        .spawn(|| {
            // Runtime is thread-affine. Construct, initialize, execute and
            // close it on the selected thread instead of moving an owner in.
            let mut runtime = Runtime::new();
            runtime
                .parts_mut()
                .unwrap()
                .with_context_gc(lua_stdlib::open_all);
            let chunk = Parser::new(SOURCE).parse().expect("fixture should parse");
            let proto = {
                let mut parts = runtime.parts_mut().unwrap();
                let (_, gc, strings) = parts.split_mut();
                let proto = CodeGenerator::new_with_pool(gc, strings)
                    .generate(&chunk, "=recursive-host-thread")
                    .expect("fixture should compile");
                gc.create(proto)
            };
            let mut returned = Vec::new();
            runtime
                .execute_proto_with_args(proto, Vec::new(), |state, values| {
                    let [Value::String(value)] = values else {
                        panic!("fixture should return its continuation marker")
                    };
                    returned = state.copy_string_bytes(*value).unwrap();
                })
                .expect("fixture should survive recursive execution");
            assert_eq!(returned, b"recursive boundary returned");
            let report = runtime
                .close()
                .expect("Runtime should close on its owner thread");
            assert_eq!(report.remaining_objects, 0);
            assert_eq!(report.remaining_allocator_live_bytes, 0);
            assert_eq!(report.runtime_resource_drain_errors, 0);
        })
        .expect("selected host thread should start")
        .join()
        .expect("selected host thread should return without panic");
    println!("host thread completed");
}
