//! One real child, no descendants. Independent of package-level fixture/locator modules.
use super::model::{Boundary, Lifecycle, Stop};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub enum Termination {
    Exited(ExitStatus),
    TimedOut,
}

pub struct RunSpec<'a> {
    pub binary: &'a Path,
    pub args: &'a [String],
    pub stdin: &'a [u8],
    pub hold_stdin: bool,
    pub timeout: Duration,
    pub ready_prefix: Option<&'a [u8]>,
}

#[derive(Debug)]
pub struct Capture {
    pub termination: Termination,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub elapsed: Duration,
    pub ready_seen: bool,
    pub alive_at_timeout: bool,
    pub hook_boundary: Option<Boundary>,
    pub backstop_steps: Vec<&'static str>,
    pub cleanup_status: Option<ExitStatus>,
    pub cleanup_error: Option<String>,
    pub final_owned: bool,
}

struct ChildOwner {
    child: Option<Child>,
    steps: Vec<&'static str>,
    waited: bool,
    status: Option<ExitStatus>,
}

impl Lifecycle for ChildOwner {
    fn kill(&mut self) -> Result<(), String> {
        self.steps.push("kill");
        self.child
            .as_mut()
            .ok_or("child already reaped")?
            .kill()
            .map_err(|error| error.to_string())
    }
    fn wait(&mut self) -> Result<(), String> {
        self.steps.push("wait");
        let status = self
            .child
            .as_mut()
            .ok_or("child already reaped")?
            .wait()
            .map_err(|error| error.to_string())?;
        self.waited = true;
        self.status = Some(status);
        self.child.take();
        Ok(())
    }
}

impl ChildOwner {
    fn backstop(&mut self) -> Result<(), String> {
        if self.child.is_some() {
            let killed = self.kill();
            self.wait()
                .map_err(|error| format!("backstop wait after kill ({killed:?}): {error}"))?;
        }
        Ok(())
    }
}

type Reader = JoinHandle<Result<Vec<u8>, String>>;
struct Guard {
    owner: ChildOwner,
    stdout: Option<Reader>,
    stderr: Option<Reader>,
    writer: Option<JoinHandle<Result<(), String>>>,
    release: Option<mpsc::Sender<()>>,
}

// ANCHOR: timeout_real_guard
impl Drop for Guard {
    fn drop(&mut self) {
        // Custom supervision, not std::process::Child's Drop behavior.
        let _ = self.owner.backstop();
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
        if let Some(stdout) = self.stdout.take() {
            let _ = stdout.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}
// ANCHOR_END: timeout_real_guard

fn drain(
    mut pipe: impl Read,
    ready: Option<(Vec<u8>, Arc<AtomicBool>)>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let len = pipe.read(&mut chunk).map_err(|error| error.to_string())?;
        if len == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..len]);
        if let Some((prefix, flag)) = &ready {
            if bytes.starts_with(prefix) {
                flag.store(true, Ordering::Release);
            }
        }
    }
}

fn joined(reader: Option<Reader>, label: &str) -> Result<Vec<u8>, String> {
    reader
        .ok_or_else(|| format!("missing {label} reader"))?
        .join()
        .map_err(|_| format!("{label} reader panicked"))?
}

fn spawn(spec: &RunSpec<'_>) -> Result<(Guard, Arc<AtomicBool>), String> {
    let mut child = std::process::Command::new(spec.binary)
        .args(spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn {}: {error}", spec.binary.display()))?;
    let ready = Arc::new(AtomicBool::new(false));
    let prefix = spec
        .ready_prefix
        .map(|prefix| (prefix.to_vec(), ready.clone()));
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut input = child.stdin.take().unwrap();
    let bytes = spec.stdin.to_vec();
    let hold = spec.hold_stdin;
    let hold_lease = spec.timeout.saturating_add(Duration::from_secs(10));
    let (release, released) = mpsc::channel();
    let guard = Guard {
        owner: ChildOwner {
            child: Some(child),
            steps: vec![],
            waited: false,
            status: None,
        },
        stdout: Some(thread::spawn(move || drain(stdout, prefix))),
        stderr: Some(thread::spawn(move || drain(stderr, None))),
        writer: Some(thread::spawn(move || {
            let result = input.write_all(&bytes).map_err(|error| error.to_string());
            if hold {
                let _ = released.recv_timeout(hold_lease);
            }
            drop(input); // Safety lease also releases the known stdin-waiting fixture.
            result
        })),
        release: Some(release),
    };
    Ok((guard, ready))
}

// ANCHOR: timeout_real_capture
pub fn capture(spec: RunSpec<'_>, stop: Stop) -> Result<Capture, String> {
    let started = Instant::now();
    let (mut guard, ready) = spawn(&spec)?;
    let needs_ready = spec.ready_prefix.is_some();
    let startup_deadline = started + Duration::from_secs(5);
    while needs_ready && !ready.load(Ordering::Acquire) {
        if Instant::now() >= startup_deadline {
            return Err("child did not emit READY within five seconds".to_owned());
        }
        thread::sleep(Duration::from_millis(2));
    }
    let deadline = Instant::now() + spec.timeout;
    let termination = loop {
        if let Some(status) = guard
            .owner
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .map_err(|e| e.to_string())?
        {
            guard.owner.steps.push("try_wait Some");
            guard.owner.child.take(); // Some already reaped; do not call wait again.
            break Termination::Exited(status);
        }
        if Instant::now() >= deadline {
            break Termination::TimedOut;
        }
        thread::sleep(Duration::from_millis(2));
    };
    finish(
        guard,
        termination,
        ready.load(Ordering::Acquire),
        started,
        stop,
    )
}
// ANCHOR_END: timeout_real_capture

fn finish(
    mut guard: Guard,
    termination: Termination,
    ready_seen: bool,
    started: Instant,
    stop: Stop,
) -> Result<Capture, String> {
    // ANCHOR: timeout_real_boundary
    let timed_out = matches!(termination, Termination::TimedOut);
    let cleanup_error = if timed_out {
        stop(&mut guard.owner).err()
    } else {
        None
    };
    let alive_probe = if timed_out && !guard.owner.steps.contains(&"kill") {
        match guard.owner.child.as_mut() {
            Some(child) => Some(
                child
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_none(),
            ),
            None => Some(false),
        }
    } else {
        None
    };
    let hook_boundary = timed_out.then(|| Boundary {
        steps: guard.owner.steps.clone(),
        owned: guard.owner.child.is_some(),
        wait_succeeded: guard.owner.waited,
        alive_probe,
    });
    let previous = guard.owner.steps.len();
    guard.owner.backstop()?; // Repair fixed faults before readers join or ownership is dropped.
    let backstop_steps = guard.owner.steps[previous..].to_vec();
    // ANCHOR_END: timeout_real_boundary
    if let Some(release) = guard.release.take() {
        let _ = release.send(());
    }
    let stdout = joined(guard.stdout.take(), "stdout")?;
    let stderr = joined(guard.stderr.take(), "stderr")?;
    guard
        .writer
        .take()
        .unwrap()
        .join()
        .map_err(|_| "stdin writer panicked")??;
    Ok(Capture {
        termination,
        stdout,
        stderr,
        elapsed: started.elapsed(),
        ready_seen,
        alive_at_timeout: timed_out,
        hook_boundary,
        backstop_steps,
        cleanup_status: guard.owner.status,
        cleanup_error,
        final_owned: guard.owner.child.is_some(),
    })
}
