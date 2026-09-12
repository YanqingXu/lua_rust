//! Windows-only compatibility for the pinned C++ exact-"ls" preflight.
//! Each child gets its own PATH and a kill-on-close job before source input.
#![cfg(windows)]

use std::ffi::{OsString, c_void};
use std::fs;
use std::io::{self, Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const LS_CMD: &[u8] = b"@echo off\r\nif \"%POPEN_TEST_WRITE%\"==\"1\" goto write\r\necho popen-available\r\nexit /b 0\r\n:write\r\nset /p payload=\r\n>pipe-write.txt echo %payload%\r\nexit /b 0\r\n";

// These repr(C) layouts and constants match Windows SDK winnt.h/jobapi2.h:
// https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information
// https://learn.microsoft.com/windows/win32/api/winnt/ns-winnt-jobobject_basic_limit_information
#[repr(C)]
#[derive(Default)]
struct BasicLimit {
    process_time: i64,
    job_time: i64,
    flags: u32,
    minimum_working_set: usize,
    maximum_working_set: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

#[repr(C)]
#[derive(Default)]
struct ExtendedLimit {
    basic: BasicLimit,
    io_counters: [u64; 6],
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(std::mem::size_of::<BasicLimit>() == 64);
    assert!(std::mem::size_of::<ExtendedLimit>() == 144);
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> RawHandle;
    fn SetInformationJobObject(
        job: RawHandle,
        information_class: i32,
        information: *const c_void,
        size: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: RawHandle, process: RawHandle) -> i32;
    fn TerminateJobObject(job: RawHandle, exit_code: u32) -> i32;
}

struct ChildJob(OwnedHandle);

impl ChildJob {
    fn new() -> io::Result<Self> {
        // SAFETY: null optional attributes/name create an unnamed job. A
        // non-null return is a new owned handle, transferred exactly once.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a valid handle owned by this call.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits = ExtendedLimit::default();
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        // SAFETY: handle remains live; repr(C) buffer/size match information
        // class 9 (JobObjectExtendedLimitInformation) and outlive this call.
        if unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                9,
                (&limits as *const ExtendedLimit).cast(),
                std::mem::size_of::<ExtendedLimit>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    fn assign(&self, child: &Child) -> io::Result<()> {
        // SAFETY: both handles remain live. The child is blocked reading
        // stdin, so it cannot create a pipe process before job assignment.
        if unsafe { AssignProcessToJobObject(self.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn terminate(&self) -> io::Result<()> {
        // SAFETY: the job handle remains live and owns only this test's child
        // and descendants; terminating it does not affect the test process.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 124) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

struct ChildTree {
    child: Child,
    job: ChildJob,
}

impl Drop for ChildTree {
    fn drop(&mut self) {
        let _ = self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
        // OwnedHandle closes the job as a final kill-on-close fallback.
    }
}

struct ScratchDirectory {
    root: PathBuf,
    path: PathBuf,
}

impl ScratchDirectory {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = root.join(format!(
            "lua-popen-{}-{stamp}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(path.is_absolute() && path.parent() == Some(root.as_path()));
        fs::create_dir(&path).unwrap();
        let path = path.canonicalize().unwrap();
        assert_eq!(path.parent(), Some(root.as_path()));
        Self { root, path }
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        if let Ok(resolved) = self.path.canonicalize()
            && resolved == self.path
            && resolved.is_absolute()
            && resolved.parent() == Some(self.root.as_path())
        {
            let _ = fs::remove_dir_all(resolved);
        }
    }
}

fn shell_path(path: &Path) -> PathBuf {
    // cmd/PATH use DOS path syntax, not canonicalize's verbatim namespace.
    // Keep the original canonical path for ownership/removal checks.
    let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
    if wide.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
        wide.splice(..8, [92, 92]);
    } else if wide.starts_with(&[92, 92, 63, 92]) {
        wide.drain(..4);
    }
    PathBuf::from(OsString::from_wide(&wide))
}

#[derive(Clone, Copy)]
enum ShellProfile {
    System,
    BasenameOnlyOnPath,
    ExistingRelative,
    Missing,
    Empty,
}

fn run_process(source: &[u8], available: bool, write_mode: bool, shell_reference: bool) -> Output {
    run_with_shell(
        source,
        available,
        write_mode,
        shell_reference,
        ShellProfile::System,
    )
}

fn run_with_shell(
    source: &[u8],
    available: bool,
    write_mode: bool,
    shell_reference: bool,
    shell: ShellProfile,
) -> Output {
    let scratch = ScratchDirectory::new();
    let system32 = shell_path(
        &PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .canonicalize()
            .unwrap(),
    );
    let command_shell = system32.join("cmd.exe");
    let path = if available {
        let bin = scratch.path.join("bin");
        fs::create_dir(&bin).unwrap();
        fs::write(bin.join("ls.cmd"), LS_CMD).unwrap();
        if matches!(shell, ShellProfile::BasenameOnlyOnPath) {
            fs::copy(system32.join("where.exe"), bin.join("custom-shell.exe")).unwrap();
        }
        std::env::join_paths([shell_path(&bin), system32.clone()]).unwrap()
    } else {
        system32.clone().into_os_string()
    };
    let comspec = match shell {
        ShellProfile::System => command_shell.clone().into_os_string(),
        ShellProfile::Missing | ShellProfile::Empty => OsString::new(),
        ShellProfile::BasenameOnlyOnPath => OsString::from("custom-shell.exe"),
        ShellProfile::ExistingRelative => {
            fs::copy(
                system32.join("where.exe"),
                scratch.path.join("custom-shell.exe"),
            )
            .unwrap();
            OsString::from(".\\custom-shell.exe")
        }
    };
    let mut command = if shell_reference {
        let mut command = Command::new(&command_shell);
        // Read the handshake before launching the missing command, just as
        // the Lua CLI reads all source before executing a popen call.
        let body = if matches!(shell, ShellProfile::ExistingRelative) {
            // Escape redirection for this reference shell: the non-shell
            // executable receives the same literal argv as the preflight.
            "set /p LuaPopenProbeReady= >NUL & .\\custom-shell.exe /c where ls ^>NUL 2^>NUL"
        } else {
            "set /p LuaPopenProbeReady= >NUL & ls "
        };
        command.args(["/C", body]);
        command
    } else {
        let mut command = Command::new(env!("CARGO_BIN_EXE_lua_app"));
        command.arg("-");
        command
    };
    command
        .current_dir(shell_path(&scratch.path))
        .env("PATH", path)
        .env("COMSPEC", comspec)
        .env("PATHEXT", ".COM;.EXE;.BAT;.CMD")
        .env("POPEN_TEST_WRITE", if write_mode { "1" } else { "0" })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    if matches!(shell, ShellProfile::Missing) {
        command.env_remove("COMSPEC");
    }
    for (name, _) in std::env::vars_os() {
        if name
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("LUA_INIT")
        {
            command.env_remove(name);
        }
    }
    let job = ChildJob::new().expect("isolated process job should be created");
    let mut tree = ChildTree {
        child: command.spawn().unwrap(),
        job,
    };
    tree.job
        .assign(&tree.child)
        .expect("child should enter its job before input");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut stdin = tree.child.stdin.take().unwrap();
    let mut stdout = tree.child.stdout.take().unwrap();
    let mut stderr = tree.child.stderr.take().unwrap();
    let input = source.to_vec();
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
        if let Some(status) = tree.child.try_wait().unwrap()
            && writer.is_finished()
            && stdout_reader.is_finished()
            && stderr_reader.is_finished()
        {
            break (status, false);
        }
        if Instant::now() >= deadline {
            tree.job
                .terminate()
                .expect("timed-out process tree should stop");
            break (tree.child.wait().unwrap(), true);
        }
        thread::sleep(Duration::from_millis(10));
    };
    // A shell descendant can hold stdout/stderr after Lua exits. The deadline
    // above includes pipe completion; terminate the whole job before joining.
    let cleanup_deadline = Instant::now() + Duration::from_secs(5);
    while !(writer.is_finished() && stdout_reader.is_finished() && stderr_reader.is_finished()) {
        assert!(
            Instant::now() < cleanup_deadline,
            "pipes stayed open after job termination"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let output = Output {
        status,
        stdout: stdout_reader.join().unwrap().unwrap(),
        stderr: stderr_reader.join().unwrap().unwrap(),
    };
    let write_result = writer.join().unwrap();
    assert!(!timed_out, "popen process exceeded 30 seconds: {output:?}");
    write_result.expect("all input bytes should be delivered");
    output
}

fn assert_output(output: Output, stdout: &str, stderr: &[u8]) {
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        output.stdout,
        stdout.replace('\n', "\r\n").as_bytes(),
        "{output:?}"
    );
    assert_eq!(output.stderr, stderr, "{output:?}");
}

#[test]
fn missing_exact_ls_raises_at_the_call_site_without_shell_noise() {
    let source = br#"for _, mode in ipairs({"r", "w"}) do
  local ok, value = pcall(io.popen, "ls", mode)
  print("open-" .. mode, ok, type(value))
  if ok then
    if mode == "r" then print("read", value:read("*a")) end
    print("close", value:close())
  else print("error", value) end
end
collectgarbage("collect")
local f = assert(io.popen("echo recovered"))
print("builtin", f:read("*l")); print("builtin-close", f:close())
print("after")
"#;
    assert_output(
        run_process(source, false, false, false),
        "open-r\tfalse\tstring\nerror\tstdin:2: io.popen: command not available\nopen-w\tfalse\tstring\nerror\tstdin:2: io.popen: command not available\nbuiltin\trecovered\nbuiltin-close\ttrue\nafter\n",
        b"",
    );
}

#[test]
fn trailing_space_bypasses_preflight_and_preserves_close_failure() {
    let shell = run_process(b"ready\n", false, false, true);
    assert_eq!(shell.status.code(), Some(1), "{shell:?}");
    assert!(shell.stdout.is_empty(), "{shell:?}");
    assert!(
        !shell.stderr.is_empty(),
        "missing command must produce shell output"
    );
    // Compare every native shell byte against the same isolated cmd environment.
    // This preserves localized output without hardcoding an English message.
    assert_output(
        run_process(
            b"local f = assert(io.popen(\"ls \"))\nprint(\"read\", f:read(\"*a\")); print(\"close\", f:close())\ncollectgarbage(\"collect\"); print(\"after\")\n",
            false, false, false,
        ),
        "read\t\nclose\tnil\tNo error\t0\nafter\n",
        &shell.stderr,
    );
}

#[test]
fn available_ls_reads_in_both_spellings_and_closes_before_gc() {
    assert_output(
        run_process(
            br#"for _, command in ipairs({"ls", "ls "}) do
  local f = assert(io.popen(command, "r"))
  print("read", f:read("*l")); print("close", f:close())
end
collectgarbage("collect"); print("after")
"#,
            true,
            false,
            false,
        ),
        "read\tpopen-available\nclose\ttrue\nread\tpopen-available\nclose\ttrue\nafter\n",
        b"",
    );
}

#[test]
fn available_ls_accepts_write_mode_and_reaps_the_consumer() {
    assert_output(
        run_process(
            br#"local f = assert(io.popen("ls", "w"))
assert(f:write("payload\n")); print("close", f:close())
local result = assert(io.open("pipe-write.txt", "r"))
print("written", result:read("*l")); assert(result:close())
collectgarbage("collect"); print("after")
"#,
            true,
            true,
            false,
        ),
        "close\ttrue\nwritten\tpayload\nafter\n",
        b"",
    );
}

#[test]
fn invalid_mode_is_rejected_before_checking_command_availability() {
    // The existing Rust mode error lacks the C++ call-site prefix. Keep that
    // separate observed behavior while checking validation precedes preflight.
    assert_output(
        run_process(b"local ok, diagnostic = pcall(io.popen, \"ls\", \"invalid\")\nprint(\"invalid\", ok, diagnostic)\ncollectgarbage(\"collect\"); print(\"after\")\n", false, false, false),
        "invalid\tfalse\tio.popen: invalid mode (must be 'r' or 'w')\nafter\n",
        b"",
    );
}

#[test]
fn comspec_basename_is_not_searched_on_path_before_system_shell_fallback() {
    // UCRT's first spawnv attempt uses the working directory, not PATH. A
    // non-shell executable present only on PATH must not suppress the fallback.
    assert_output(
        run_with_shell(
            b"local f=assert(io.popen('ls')); print(f:read('*l')); print(f:close())\n",
            true,
            false,
            false,
            ShellProfile::BasenameOnlyOnPath,
        ),
        "popen-available\ntrue\n",
        b"",
    );
}

#[test]
fn existing_relative_comspec_nonzero_exit_does_not_fall_back() {
    // This copy starts but rejects /c. Compare its native output with the same
    // executable/argv: a non-shell COMSPEC does not interpret redirection.
    let reference = run_with_shell(
        b"ready\n",
        true,
        false,
        true,
        ShellProfile::ExistingRelative,
    );
    assert_eq!(reference.status.code(), Some(2), "{reference:?}");
    let output = run_with_shell(
            b"local ok,e=pcall(io.popen,'ls'); print(ok,e); collectgarbage('collect'); print('after')\n",
            true, false, false, ShellProfile::ExistingRelative,
    );
    let mut expected_stdout = reference.stdout;
    expected_stdout
        .extend_from_slice(b"false\tstdin:1: io.popen: command not available\r\nafter\r\n");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected_stdout, "{output:?}");
    assert_eq!(output.stderr, reference.stderr, "{output:?}");
}

#[test]
fn missing_comspec_uses_default_shell_and_empty_comspec_is_controlled() {
    assert_output(
        run_with_shell(
            b"local f=assert(io.popen('ls')); print(f:read('*l')); print(f:close())\n",
            true,
            false,
            false,
            ShellProfile::Missing,
        ),
        "popen-available\ntrue\n",
        b"",
    );
    // Pinned C++ aborts for empty COMSPEC. This explicitly preserves Rust's
    // controlled diagnostic, GC, and subsequent execution rather than aborting.
    assert_output(
        run_with_shell(
            b"local ok,e=pcall(io.popen,'ls'); print(ok,e); collectgarbage('collect'); print('after')\n",
            true, false, false, ShellProfile::Empty,
        ),
        "false\tstdin:1: io.popen: command not available\nafter\n",
        b"",
    );
}
