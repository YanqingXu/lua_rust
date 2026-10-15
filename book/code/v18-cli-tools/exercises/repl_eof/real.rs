//! These checks launch the actual CLI; the old shared runner closes stdin.
use crate::process;
use std::path::Path;

const BANNER: &str = "Lua 5.1 (lua-rust teaching v0.18)\n";

#[derive(Debug)]
pub struct Case {
    pub name: &'static str,
    pub exit: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

// ANCHOR: repl_eof_real
fn check(
    binary: &Path,
    name: &'static str,
    input: &[u8],
    exit: i32,
    body: &str,
    diagnostics: &[&str],
) -> Result<Case, String> {
    let capture = process::run(binary, &["-i".to_owned()], input)?;
    let expected_stdout = format!("{BANNER}{body}");
    if capture.status.code() != Some(exit) || capture.stdout != expected_stdout.as_bytes() {
        return Err(format!(
            "{name}: expected exit={exit}, stdout={expected_stdout:?}, got {capture:?}"
        ));
    }
    let stderr = String::from_utf8_lossy(&capture.stderr);
    if stderr.matches("lua: ").count() != diagnostics.len()
        || !diagnostics.iter().all(|message| stderr.contains(message))
    {
        return Err(format!(
            "{name}: expected diagnostics {diagnostics:?}, got {stderr:?}"
        ));
    }
    Ok(Case {
        name,
        exit,
        stdout: capture.stdout,
        stderr: capture.stderr,
    })
}
// ANCHOR_END: repl_eof_real

pub fn eof_cases(binary: &Path) -> Result<Vec<Case>, String> {
    Ok(vec![
        check(binary, "idle EOF", b"", 0, "> \n", &[])?,
        check(binary, "blank then idle EOF", b"  \n", 0, "> > \n", &[])?,
        check(
            binary,
            "last line has no LF",
            b"=40+2",
            0,
            "> 42\n> \n",
            &[],
        )?,
        check(
            binary,
            "short string final fragment has EOF instead of LF",
            b"return 'broken",
            1,
            "> >> \n",
            &["1:15: unterminated short string"],
        )?,
        check(
            binary,
            "expression pending EOF",
            b"return 40 +\n",
            1,
            "> >> \n",
            &["expected an expression"],
        )?,
        check(
            binary,
            "block pending EOF has no prefix side effect",
            b"print('must-not-run'); if true then\n",
            1,
            "> >> \n",
            &["expected 'end'"],
        )?,
        check(
            binary,
            "earlier chunks remain visible before pending EOF",
            b"saved=40\n=saved+2\nif true then\n",
            1,
            "> > 42\n> >> \n",
            &["expected 'end'"],
        )?,
        check(
            binary,
            "ordinary syntax error recovers before idle EOF",
            b"local =\n=42\n",
            0,
            "> > 42\n> \n",
            &["expected a variable name"],
        )?,
        check(
            binary,
            "short string newline is invalid and recovers",
            b"return 'broken\n=42\n",
            0,
            "> > 42\n> \n",
            &["string"],
        )?,
        check(
            binary,
            "long string pending EOF",
            b"return [[unfinished\n",
            1,
            "> >> \n",
            &["unterminated"],
        )?,
    ])
}
