//! 实际二进制 -i/-e/stdin 对照；共享 runner 负责 EOF、管道和有界等待。
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

fn check(
    binary: &Path,
    name: &'static str,
    args: &[&str],
    input: &[u8],
    expected_exit: i32,
    expected_stdout: &str,
    errors: &[&str],
) -> Result<Case, String> {
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let output = process::run(binary, &args, input)?;
    if output.status.code() != Some(expected_exit) || output.stdout != expected_stdout.as_bytes() {
        return Err(format!(
            "{name}: expected exit={expected_exit}, stdout={expected_stdout:?}; got {output:?}"
        ));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.matches("lua: ").count() != errors.len()
        || !errors.iter().all(|message| stderr.contains(message))
    {
        return Err(format!("{name}: unexpected error stream {stderr:?}"));
    }
    Ok(Case {
        name,
        exit: expected_exit,
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

// ANCHOR: repl_recovery_real_interactive
pub fn interactive_recovery(binary: &Path) -> Result<Vec<Case>, String> {
    Ok(vec![
        check(
            binary,
            "interactive recovery",
            &["-i"],
            b"saved=40\nlocal =\n=saved+2\neffects=1;error('recover')\n=effects\n=saved\n",
            0,
            &format!("{BANNER}> > > 42\n> > 1\n> 40\n> \n"),
            &["expected a variable name", "recover"],
        )?,
        check(
            binary,
            "nil versus no values",
            &["-i"],
            b"return nil\nreturn\n=42\n",
            0,
            &format!("{BANNER}> nil\n> > 42\n> \n"),
            &[],
        )?,
    ])
}
// ANCHOR_END: repl_recovery_real_interactive

pub fn mode_boundaries(binary: &Path) -> Result<Vec<Case>, String> {
    Ok(vec![
        check(
            binary,
            "execute error exits one",
            &["-e", "error('recover')"],
            b"",
            1,
            "",
            &["recover"],
        )?,
        check(
            binary,
            "incomplete EOF silently exits zero",
            &["-i"],
            b"if true then\nprint('never')\n",
            0,
            &format!("{BANNER}> >> >> \n"),
            &[],
        )?,
        check(
            binary,
            "pipe without i is one runtime chunk",
            &[],
            b"print('first')\nerror('pipe-stop')\nprint('after')\n",
            1,
            "first\n",
            &["pipe-stop"],
        )?,
        check(
            binary,
            "pipe compiles whole chunk before first statement",
            &[],
            b"print('must-not-run')\nlocal =\n",
            1,
            "",
            &["expected a variable name"],
        )?,
        check(
            binary,
            "i executes each completed chunk then continues",
            &["-i"],
            b"print('must-not-run')\nlocal =\n",
            0,
            &format!("{BANNER}> must-not-run\n> > \n"),
            &["expected a variable name"],
        )?,
        check(
            binary,
            "heuristic misses appendable expression",
            &["-i"],
            b"return 40 +\n2\n=42\n",
            0,
            &format!("{BANNER}> > > 42\n> \n"),
            &[
                "expected an expression",
                "expected local, assignment, do block or return",
            ],
        )?,
    ])
}
