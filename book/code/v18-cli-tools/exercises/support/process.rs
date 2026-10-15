//! Bounded teaching-child runner. It owns one exact child; fixtures spawn no descendants.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct ChildOwner(Option<Child>);

impl ChildOwner {
    fn stop(&mut self) -> Result<(), String> {
        let Some(child) = self.0.as_mut() else {
            return Ok(());
        };
        // A concurrent ordinary exit may make kill fail; wait still reaps this child.
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

pub fn run(binary: &Path, args: &[String], stdin: &[u8]) -> Result<Output, String> {
    let child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn {}: {error}", binary.display()))?;
    let mut owner = ChildOwner(Some(child));
    let child = owner.0.as_mut().unwrap();
    // ANCHOR: cli_process_pipes
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut input = child.stdin.take().unwrap();
    let source = stdin.to_vec();
    let output_reader = thread::spawn(move || drain(stdout));
    let error_reader = thread::spawn(move || drain(stderr));
    let input_writer = thread::spawn(move || {
        let result = input.write_all(&source);
        drop(input); // EOF is visible even while the parent is polling the child.
        result
    });
    // ANCHOR_END: cli_process_pipes
    // ANCHOR: cli_process_deadline
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match owner.0.as_mut().unwrap().try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => break Err("child exceeded five-second deadline".to_owned()),
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
    // ANCHOR_END: cli_process_deadline
    let stdout = output_reader.join().map_err(|_| "stdout reader panicked")?;
    let stderr = error_reader.join().map_err(|_| "stderr reader panicked")?;
    let written = input_writer.join().map_err(|_| "stdin writer panicked")?;
    let status = status?;
    if let Err(error) = written {
        // Early help/usage/error exit can legitimately stop consuming input.
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(format!("stdin: {error}"));
        }
    }
    Ok(Output {
        status,
        stdout: stdout?,
        stderr: stderr?,
    })
}

// ANCHOR: cli_process_example_binary
#[allow(dead_code)] // Integration targets use env!(CARGO_BIN_EXE_lua_app) instead.
pub fn example_binary() -> Result<PathBuf, String> {
    let example = std::env::current_exe().map_err(|error| error.to_string())?;
    let profile = example
        .parent()
        .and_then(Path::parent)
        .ok_or("example has no profile directory")?;
    let name = if cfg!(windows) {
        "lua_app.exe"
    } else {
        "lua_app"
    };
    let binary = profile.join(name);
    if binary.is_file() {
        Ok(binary)
    } else {
        Err(format!("missing sibling binary {}; first run: cargo build --manifest-path book/code/v18-cli-tools/Cargo.toml --bins", binary.display()))
    }
}
// ANCHOR_END: cli_process_example_binary
