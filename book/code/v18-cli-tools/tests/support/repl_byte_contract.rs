//! Same local byte oracle in each independent snapshot. No source or build dependency on a sibling.
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub const CORPUS: &str = include_str!("../fixtures/repl-byte-contract.tsv");

#[derive(Debug)]
pub struct Case {
    pub id: String,
    pub args: Vec<String>,
    pub stdin: Vec<u8>,
    pub exit: i32,
    banner: bool,
    body: Vec<u8>,
    pub stderr: Vec<u8>,
}

fn hex(field: &str) -> Vec<u8> {
    assert_eq!(field.len() % 2, 0, "odd hex field");
    assert!(
        field
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "expected lowercase hex"
    );
    field
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

// ANCHOR: repl_contract_parse
pub fn cases() -> Vec<Case> {
    let mut ids = BTreeSet::new();
    CORPUS
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<_> = line.split('|').collect();
            assert_eq!(fields.len(), 7, "bad contract row: {line}");
            assert!(
                !fields[0].is_empty()
                    && fields[0]
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'-')
                    && ids.insert(fields[0]),
                "bad or duplicate case id"
            );
            let args = if fields[1].is_empty() {
                Vec::new()
            } else {
                fields[1]
                    .split(',')
                    .map(|arg| String::from_utf8(hex(arg)).unwrap())
                    .collect()
            };
            Case {
                id: fields[0].to_owned(),
                args,
                stdin: hex(fields[2]),
                exit: match fields[3] {
                    "0" => 0,
                    "1" => 1,
                    other => panic!("unexpected exit: {other}"),
                },
                banner: match fields[4] {
                    "none" => false,
                    "repl" => true,
                    other => panic!("unexpected banner policy: {other}"),
                },
                body: hex(fields[5]),
                stderr: hex(fields[6]),
            }
        })
        .collect()
}
// ANCHOR_END: repl_contract_parse

// ANCHOR: repl_contract_banner
pub fn banner(version: &str) -> &'static [u8] {
    match version {
        "0.18.0" => b"Lua 5.1 (lua-rust teaching v0.18)\n",
        "1.0.0" => b"Lua 5.1 (lua-rust teaching v1.0)\n",
        other => panic!("contract has no declared banner for {other}"),
    }
}

impl Case {
    pub fn stdout(&self, version: &str) -> Vec<u8> {
        let mut bytes = if self.banner {
            banner(version).to_vec()
        } else {
            Vec::new()
        };
        bytes.extend_from_slice(&self.body);
        bytes
    }
}
// ANCHOR_END: repl_contract_banner

struct ChildOwner(Option<Child>);

impl ChildOwner {
    fn stop(&mut self) -> Result<(), String> {
        let Some(child) = self.0.as_mut() else {
            return Ok(());
        };
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
    pipe.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

// ANCHOR: repl_contract_process_pipes
pub fn capture(binary: &Path, case: &Case) -> Result<Output, String> {
    let child = Command::new(binary)
        .args(&case.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn {}: {error}", binary.display()))?;
    let mut owner = ChildOwner(Some(child));
    let child = owner.0.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let mut input = child.stdin.take().unwrap();
    let source = case.stdin.clone();
    let output_reader = thread::spawn(move || drain(stdout));
    let error_reader = thread::spawn(move || drain(stderr));
    let input_writer = thread::spawn(move || {
        let result = input.write_all(&source);
        drop(input); // Closing the last writer, not flush, delivers stdin EOF.
        result
    });
    // ANCHOR_END: repl_contract_process_pipes
    // ANCHOR: repl_contract_process_deadline
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        match owner.0.as_mut().unwrap().try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => break Err(format!("{} exceeded five-second deadline", case.id)),
            Err(error) => break Err(format!("try_wait: {error}")),
        }
    };
    let status = match status {
        Ok(status) => {
            owner.0.take(); // try_wait(Some) has already reaped this exact child.
            Ok(status)
        }
        Err(error) => match owner.stop() {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; additionally {cleanup}")),
        },
    };
    // Join every thread before propagating either a process or pipe error.
    let stdout = output_reader.join();
    let stderr = error_reader.join();
    let written = input_writer.join();
    let status = status?;
    let stdout = stdout.map_err(|_| "stdout reader panicked")??;
    let stderr = stderr.map_err(|_| "stderr reader panicked")??;
    let written = written.map_err(|_| "stdin writer panicked")?;
    if let Err(error) = written {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(format!("stdin: {error}"));
        }
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}
// ANCHOR_END: repl_contract_process_deadline

// ANCHOR: repl_contract_verify
pub fn verify(binary: &Path, version: &str) {
    for case in cases() {
        let actual = capture(binary, &case).unwrap_or_else(|error| panic!("{}: {error}", case.id));
        assert_eq!(actual.status.code(), Some(case.exit), "{} exit", case.id);
        assert_eq!(
            actual.stdout,
            case.stdout(version),
            "{} stdout raw bytes",
            case.id
        );
        assert_eq!(actual.stderr, case.stderr, "{} stderr raw bytes", case.id);
    }
}
// ANCHOR_END: repl_contract_verify
