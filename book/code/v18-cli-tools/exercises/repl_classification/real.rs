//! Bounded subprocess observations of the actual updated lua_app -i command.
use crate::process;
use std::path::Path;

const BANNER: &str = "Lua 5.1 (lua-rust teaching v0.18)\n";

#[derive(Debug)]
pub struct Case {
    pub name: &'static str,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn cases(binary: &Path) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    for (name, input, suffix, diagnostic) in [
        (
            "expression resumes",
            b"return 40 +\n2\n".as_slice(),
            "> >> 42\n> \n",
            None,
        ),
        (
            "bad exponent recovers",
            b"return 1e\n=42\n",
            "> > 42\n> \n",
            Some("number exponent needs a digit"),
        ),
        (
            "short string newline recovers",
            b"return 'hello\n=42\n",
            "> > 42\n> \n",
            Some("newline in short string"),
        ),
        (
            "long comment resumes",
            b"--[=[hello\nworld]=]\n=42\n",
            "> >> > 42\n> \n",
            None,
        ),
    ] {
        let output = process::run(binary, &["-i".into()], input)?;
        let expected_stdout = format!("{BANNER}{suffix}").into_bytes();
        if output.status.code() != Some(0) || output.stdout != expected_stdout {
            return Err(format!(
                "{name}: expected exit=0 stdout={expected_stdout:?}, got {output:?}"
            ));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        match diagnostic {
            Some(message) if stderr.matches("lua: ").count() == 1 && stderr.contains(message) => {}
            None if output.stderr.is_empty() => {}
            _ => return Err(format!("{name}: unexpected stderr {stderr:?}")),
        }
        cases.push(Case {
            name,
            stdout: output.stdout,
            stderr: output.stderr,
        });
    }
    Ok(cases)
}
