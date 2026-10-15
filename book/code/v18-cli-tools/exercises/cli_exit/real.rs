//! Public child processes, independent of the synthetic Result hooks.
use std::path::Path;

use super::model::USAGE;
use crate::{process, temp::TempDirectory};

pub struct Probe {
    pub name: &'static str,
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr_class: &'static str,
}

fn check(
    binary: &Path,
    name: &'static str,
    args: &[String],
    input: &[u8],
    code: i32,
    stdout: &[u8],
    stderr_contains: &str,
) -> Result<Probe, String> {
    let output = process::run(binary, args, input)?;
    if output.status.code() != Some(code) || output.stdout != stdout {
        return Err(format!(
            "{name}: exit={:?}, stdout={:?}, stderr={:?}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if code == 0 {
        if !stderr.is_empty() {
            return Err(format!("{name}: unexpected stderr {stderr:?}"));
        }
    } else if !stderr.starts_with("lua: ")
        || !stderr.contains(stderr_contains)
        || (code == 2) != stderr.contains("usage:")
    {
        return Err(format!("{name}: wrong diagnostic stream/class {stderr:?}"));
    }
    Ok(Probe {
        name,
        code,
        stdout: output.stdout,
        stderr_class: if code == 0 {
            "empty"
        } else if code == 2 {
            "usage"
        } else {
            "failure"
        },
    })
}

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub fn help_version_and_usage(binary: &Path) -> Result<Vec<Probe>, String> {
    let mut probes = Vec::new();
    for (name, argument) in [("help", "--help"), ("short-help", "-h")] {
        probes.push(check(
            binary,
            name,
            &args(&[argument]),
            b"",
            0,
            format!("{USAGE}\n").as_bytes(),
            "",
        )?);
    }
    for (name, argument) in [("version", "--version"), ("short-version", "-v")] {
        probes.push(check(
            binary,
            name,
            &args(&[argument]),
            b"",
            0,
            b"Lua 5.1 (lua-rust teaching v0.18)\n",
            "",
        )?);
    }
    for (name, argument, message) in [
        ("unknown", "--unknown", "unrecognized option"),
        ("missing-e", "-e", "needs an argument"),
        ("missing-l", "-l", "needs an argument"),
    ] {
        probes.push(check(
            binary,
            name,
            &args(&[argument]),
            b"",
            2,
            b"",
            message,
        )?);
    }
    Ok(probes)
}

pub fn compile_runtime_file_and_silent_returns(binary: &Path) -> Result<Vec<Probe>, String> {
    let directory = TempDirectory::new("exit-matrix")?;
    let result = (|| {
        let script = directory.path("return.lua");
        std::fs::write(&script, b"print('script'); return 42")
            .map_err(|error| error.to_string())?;
        let script = script.to_string_lossy().into_owned();
        let missing = directory.path("missing.lua").to_string_lossy().into_owned();
        Ok(vec![
            check(
                binary,
                "compile",
                &args(&["-e", "local ="]),
                b"",
                1,
                b"",
                "expected",
            )?,
            check(
                binary,
                "runtime-keeps-output",
                &args(&["-e", "print('kept'); error('broken')"]),
                b"",
                1,
                b"kept\n",
                "broken",
            )?,
            check(
                binary,
                "missing-file",
                &args(&[&missing]),
                b"",
                1,
                b"",
                "cannot read",
            )?,
            check(binary, "stdin-return-silent", &[], b"return 42", 0, b"", "")?,
            check(
                binary,
                "execute-return-silent",
                &args(&["-e", "print('printed'); return 42"]),
                b"",
                0,
                b"printed\n",
                "",
            )?,
            check(
                binary,
                "script-return-silent",
                &args(&[&script]),
                b"",
                0,
                b"script\n",
                "",
            )?,
        ])
    })();
    directory.finish(result)
}

// ANCHOR: cli_exit_real_abort
pub fn failed_action_skips_later_execute_script_and_repl(
    binary: &Path,
) -> Result<Vec<Probe>, String> {
    let directory = TempDirectory::new("exit-abort")?;
    let result = (|| {
        let path = directory.path("later.lua");
        std::fs::write(&path, b"print('SCRIPT SHOULD NOT RUN')")
            .map_err(|error| error.to_string())?;
        let path = path.to_string_lossy().into_owned();
        Ok(vec![
            check(
                binary,
                "execute-aborts-actions",
                &args(&[
                    "-i",
                    "-e",
                    "print('kept'); error('action failed')",
                    "-e",
                    "print('LATER SHOULD NOT RUN')",
                    &path,
                ]),
                b"print('REPL SHOULD NOT RUN')\n",
                1,
                b"kept\n",
                "action failed",
            )?,
            check(
                binary,
                "load-aborts-actions",
                &args(&[
                    "-i",
                    "-e",
                    "print('kept'); package.preload.bad=function() error('load failed') end",
                    "-lbad",
                    "-e",
                    "print('LATER SHOULD NOT RUN')",
                    &path,
                ]),
                b"print('REPL SHOULD NOT RUN')\n",
                1,
                b"kept\n",
                "load failed",
            )?,
        ])
    })();
    directory.finish(result)
}
// ANCHOR_END: cli_exit_real_abort

pub fn failed_script_or_file_skips_repl(binary: &Path) -> Result<Vec<Probe>, String> {
    let directory = TempDirectory::new("script-abort")?;
    let result = (|| {
        let path = directory.path("failure.lua");
        std::fs::write(&path, b"print('script before'); error('script failed')")
            .map_err(|error| error.to_string())?;
        let path = path.to_string_lossy().into_owned();
        let missing = directory.path("missing.lua").to_string_lossy().into_owned();
        Ok(vec![
            check(
                binary,
                "script-aborts-repl",
                &args(&["-i", &path]),
                b"print('REPL SHOULD NOT RUN')\n",
                1,
                b"script before\n",
                "script failed",
            )?,
            check(
                binary,
                "file-aborts-repl",
                &args(&["-v", "-i", &missing]),
                b"print('REPL SHOULD NOT RUN')\n",
                1,
                b"Lua 5.1 (lua-rust teaching v0.18)\n",
                "cannot read",
            )?,
        ])
    })();
    directory.finish(result)
}
