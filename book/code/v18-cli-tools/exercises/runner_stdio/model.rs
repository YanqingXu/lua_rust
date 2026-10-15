//! Real child/pipe experiment. Hooks govern EOF delivery and reader scheduling only.
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub enum Notice {
    Ready,
    Ack,
}

pub struct Streams {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}
pub struct DrainTask {
    pub worker: JoinHandle<Result<Streams, String>>,
    pub after_wait: Option<Sender<()>>,
}

impl DrainTask {
    fn finish(self) -> Result<Streams, String> {
        if let Some(release) = self.after_wait {
            let _ = release.send(());
        }
        self.worker
            .join()
            .map_err(|_| "drain supervisor panicked".to_owned())?
    }
}

pub type SendInput = fn(ChildStdin, &[u8]) -> Result<Option<ChildStdin>, String>;
pub type StartDrains = fn(ChildStdout, ChildStderr, Sender<Notice>) -> DrainTask;

pub struct Report {
    pub timed_out: bool,
    pub ready: bool,
    pub ack: bool,
    pub killed: bool,
    pub reaped: bool,
    pub joined: bool,
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn read_pipe(mut pipe: impl Read, notices: Option<Sender<Notice>>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 8192];
    let mut ready = false;
    let mut ack = false;
    loop {
        let size = pipe.read(&mut chunk).map_err(|error| error.to_string())?;
        if size == 0 {
            return Ok(bytes);
        }
        bytes.extend_from_slice(&chunk[..size]);
        if let Some(sender) = &notices {
            let prefix = &bytes[..bytes.len().min(64)];
            if !ready && prefix.windows(6).any(|value| value == b"READY ") {
                let _ = sender.send(Notice::Ready);
                ready = true;
            }
            if !ack && prefix.windows(4).any(|value| value == b"ACK ") {
                let _ = sender.send(Notice::Ack);
                ack = true;
            }
        }
    }
}

pub fn join_pair(
    stdout: JoinHandle<Result<Vec<u8>, String>>,
    stderr: JoinHandle<Result<Vec<u8>, String>>,
) -> Result<Streams, String> {
    // Join both before propagating either error; no detached reader on early ?.
    let out = stdout.join();
    let err = stderr.join();
    Ok(Streams {
        stdout: out.map_err(|_| "stdout reader panicked")??,
        stderr: err.map_err(|_| "stderr reader panicked")??,
    })
}

pub fn sequential(stdout: ChildStdout, stderr: ChildStderr, notices: Sender<Notice>) -> DrainTask {
    DrainTask {
        worker: thread::spawn(move || {
            let stdout = read_pipe(stdout, Some(notices));
            let stderr = read_pipe(stderr, None);
            Ok(Streams {
                stdout: stdout?,
                stderr: stderr?,
            })
        }),
        after_wait: None,
    }
}

pub fn write_flush(mut stdin: ChildStdin, input: &[u8]) -> Result<ChildStdin, String> {
    stdin
        .write_all(input)
        .and_then(|()| stdin.flush())
        .map_err(|error| error.to_string())?;
    Ok(stdin)
}

struct Running {
    child: Child,
    drains: Option<DrainTask>,
    held: Option<ChildStdin>,
    reaped: bool,
}
impl Drop for Running {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.held.take();
        if let Some(drains) = self.drains.take() {
            let _ = drains.finish();
        }
    }
}

pub fn run(
    binary: &Path,
    mode: &str,
    input: &[u8],
    send: SendInput,
    start: StartDrains,
) -> Result<Report, String> {
    let child = Command::new(binary)
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut owner = Running {
        child,
        drains: None,
        held: None,
        reaped: false,
    };
    let (tx, rx) = mpsc::channel();
    let stdout = owner.child.stdout.take().unwrap();
    let stderr = owner.child.stderr.take().unwrap();
    owner.drains = Some(start(stdout, stderr, tx));
    owner.held = send(owner.child.stdin.take().unwrap(), input)?;
    // ANCHOR: runner_stdio_supervisor
    let deadline = Instant::now() + Duration::from_secs(2);
    let (mut ready, mut ack, mut killed, mut timed_out) = (false, false, false, false);
    let status = loop {
        for notice in rx.try_iter() {
            match notice {
                Notice::Ready => ready = true,
                Notice::Ack => ack = true,
            }
        }
        if let Some(status) = owner.child.try_wait().map_err(|error| error.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            killed = owner.child.kill().is_ok();
            break owner.child.wait().map_err(|error| error.to_string())?;
        }
        thread::sleep(Duration::from_millis(5));
    };
    owner.reaped = true;
    owner.held.take();
    let captured = owner.drains.take().unwrap().finish()?;
    for notice in rx.try_iter() {
        match notice {
            Notice::Ready => ready = true,
            Notice::Ack => ack = true,
        }
    }
    Ok(Report {
        timed_out,
        ready,
        ack,
        killed,
        reaped: true,
        joined: true,
        status,
        stdout: captured.stdout,
        stderr: captured.stderr,
    })
}
// ANCHOR_END: runner_stdio_supervisor
