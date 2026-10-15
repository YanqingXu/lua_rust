use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_BUILD: AtomicUsize = AtomicUsize::new(0);

pub struct Compiled {
    pub output: Output,
    executable: PathBuf,
    log_dir: PathBuf,
}

pub fn compile(case: &str) -> Compiled {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = manifest.join("exercises/borrow").join(format!("{case}.rs"));
    let serial = NEXT_BUILD.fetch_add(1, Ordering::Relaxed);
    let log_dir = manifest
        .join("../../../target/teaching-v14-callbacks-2026-10-15/borrow")
        .join(format!("{}-{serial}-{case}", std::process::id()));
    fs::create_dir_all(&log_dir).expect("create isolated rustc output directory");
    let executable = log_dir.join(format!("{case}{}", std::env::consts::EXE_SUFFIX));
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .arg("--edition=2021")
        .arg("--crate-name")
        .arg(format!("borrow_{case}"))
        .arg("--error-format=json")
        .arg("-Dwarnings")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("run local rustc (no downloads or external test dependencies)");
    fs::write(log_dir.join("rustc.stderr.jsonl"), &output.stderr).unwrap();
    fs::write(log_dir.join("rustc.stdout.log"), &output.stdout).unwrap();
    Compiled {
        output,
        executable,
        log_dir,
    }
}

pub fn assert_compiles_and_scans(case: &str) {
    let compiled = compile(case);
    assert!(
        compiled.output.status.success(),
        "{case} must compile before its behavior can be checked:\n{}",
        String::from_utf8_lossy(&compiled.output.stderr)
    );
    let run = Command::new(&compiled.executable)
        .output()
        .expect("run graph model");
    fs::write(compiled.log_dir.join("run.stdout.log"), &run.stdout).unwrap();
    fs::write(compiled.log_dir.join("run.stderr.log"), &run.stderr).unwrap();
    assert!(
        run.status.success(),
        "graph assertions failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stderr.is_empty(), "graph model must not emit errors");
    let actual = String::from_utf8(run.stdout).unwrap().replace("\r\n", "\n");
    assert_eq!(
        actual,
        "after parent: gray=[1, 2] marked=[true, false, false, false]\nafter drain: gray=[] marked=[true, true, true, false]\n"
    );
    println!("{case}: rustc accepted; runtime graph assertions passed\n{actual}");
}
