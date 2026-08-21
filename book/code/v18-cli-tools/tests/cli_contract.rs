use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);

fn run_process(executable: &str, arguments: &[String], stdin: &[u8], cwd: Option<&Path>) -> Output {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("process exceeded five-second timeout: {arguments:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut stdout)
        .unwrap();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    Output {
        status,
        stdout,
        stderr,
    }
}

fn temp_directory(label: &str) -> PathBuf {
    let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "lua_rust_v18_{label}_{}_{}",
        std::process::id(),
        id
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

fn decode_field(field: &str) -> String {
    field.replace("\\t", "\t").replace("\\n", "\n")
}

#[test]
fn fixture_manifest_checks_stdout_stderr_exit_and_arguments() {
    let executable = env!("CARGO_BIN_EXE_lua_app");
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let manifest = std::fs::read_to_string(fixture_root.join("manifest.tsv")).unwrap();
    for line in manifest.lines().filter(|line| !line.starts_with('#')) {
        let fields = line.split('|').collect::<Vec<_>>();
        assert_eq!(fields.len(), 6, "bad manifest row: {line}");
        let mut arguments = vec![fixture_root.join(fields[1]).to_string_lossy().into_owned()];
        arguments.extend(
            fields[2]
                .split(',')
                .filter(|argument| !argument.is_empty())
                .map(str::to_owned),
        );
        let output = run_process(executable, &arguments, b"", None);
        assert_eq!(
            output.status.code(),
            Some(fields[3].parse().unwrap()),
            "fixture {}",
            fields[0]
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            decode_field(fields[4]),
            "fixture {}",
            fields[0]
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(fields[5]),
            "fixture {} stderr was {}",
            fields[0],
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn execute_load_double_dash_and_stdin_modes_share_one_session() {
    let executable = env!("CARGO_BIN_EXE_lua_app");
    let execute = run_process(
        executable,
        &[
            "-e".to_owned(),
            "answer = 40".to_owned(),
            "-eprint(answer + 2)".to_owned(),
        ],
        b"",
        None,
    );
    assert!(execute.status.success());
    assert_eq!(execute.stdout, b"42\n");

    let stdin = run_process(executable, &[], b"print(6 * 7)", None);
    assert!(stdin.status.success());
    assert_eq!(stdin.stdout, b"42\n");

    let directory = temp_directory("load");
    std::fs::write(
        directory.join("prelude.lua"),
        b"loaded_answer = 42\nreturn true",
    )
    .unwrap();
    std::fs::write(directory.join("-script.lua"), b"print(loaded_answer)").unwrap();
    let loaded = run_process(
        executable,
        &[
            "-lprelude".to_owned(),
            "--".to_owned(),
            "-script.lua".to_owned(),
        ],
        b"",
        Some(&directory),
    );
    assert!(
        loaded.status.success(),
        "{}",
        String::from_utf8_lossy(&loaded.stderr)
    );
    assert_eq!(loaded.stdout, b"42\n");
    std::fs::remove_file(directory.join("prelude.lua")).unwrap();
    std::fs::remove_file(directory.join("-script.lua")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}

#[test]
fn repl_keeps_state_handles_expression_shorthand_and_multiline_chunks() {
    let output = run_process(
        env!("CARGO_BIN_EXE_lua_app"),
        &["-i".to_owned()],
        b"answer = 40\n=answer + 2\nif true then\nprint('multi')\nend\n",
        None,
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("Lua 5.1 (lua-rust teaching v0.18)\n> "));
    assert!(stdout.contains("42\n> "));
    assert!(stdout.contains(">> multi\n"));
}

#[test]
fn usage_and_runtime_failures_use_distinct_exit_codes() {
    let executable = env!("CARGO_BIN_EXE_lua_app");
    let usage = run_process(executable, &["--unknown".to_owned()], b"", None);
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("usage:"));

    let runtime = run_process(
        executable,
        &["-e".to_owned(), "error('broken')".to_owned()],
        b"",
        None,
    );
    assert_eq!(runtime.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&runtime.stderr).contains("broken"));
}

#[test]
fn script_side_effect_is_visible_after_process_exit() {
    let directory = temp_directory("side_effect");
    let output_path = directory.join("effect.txt");
    let script_path = directory.join("write.lua");
    let lua_output_path = output_path.to_string_lossy().replace('\\', "/");
    std::fs::write(
        &script_path,
        format!(
            "local file = assert(io.open('{lua_output_path}', 'w'))\nfile:write('done')\nfile:close()"
        ),
    )
    .unwrap();
    let output = run_process(
        env!("CARGO_BIN_EXE_lua_app"),
        &[script_path.to_string_lossy().into_owned()],
        b"",
        None,
    );
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(std::fs::read(&output_path).unwrap(), b"done");
    std::fs::remove_file(output_path).unwrap();
    std::fs::remove_file(script_path).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
