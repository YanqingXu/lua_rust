use lua_core::Value;

use super::model::{CliError, Finish, Outcome, Transcript, USAGE};

// ANCHOR: cli_exit_contract
pub fn result_boundary(finish: Finish) {
    for stdout in [Vec::new(), b"already printed\n".to_vec(), vec![0, 255]] {
        for values in [
            vec![],
            vec![Value::Nil],
            vec![Value::Number(42.0), Value::Nil],
        ] {
            assert_eq!(
                finish(Outcome {
                    stdout: stdout.clone(),
                    result: Ok(values)
                }),
                Transcript {
                    code: 0,
                    stdout: stdout.clone(),
                    stderr: vec![]
                }
            );
        }
        for (error, code, stderr) in [
            (
                CliError::Usage("unknown option".to_owned()),
                2,
                format!("lua: unknown option\n{USAGE}\n"),
            ),
            (
                CliError::Runtime("line 1: failed".to_owned()),
                1,
                "lua: line 1: failed\n".to_owned(),
            ),
        ] {
            assert_eq!(
                finish(Outcome {
                    stdout: stdout.clone(),
                    result: Err(error)
                }),
                Transcript {
                    code,
                    stdout: stdout.clone(),
                    stderr: stderr.into_bytes()
                }
            );
        }
    }
}
// ANCHOR_END: cli_exit_contract
