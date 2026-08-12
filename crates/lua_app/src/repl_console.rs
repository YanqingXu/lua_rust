//! Native console line editing for the full Windows REPL.

use lua_vm::runtime::Runtime;

pub(crate) enum ConsoleRead {
    Line(String),
    Interrupted,
    Eof,
}

#[cfg(windows)]
mod platform {
    use super::{ConsoleRead, Runtime};
    use std::ffi::c_void;

    const STD_INPUT_HANDLE: u32 = (-10_i32) as u32;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(standard_handle: u32) -> *mut c_void;
        fn GetConsoleMode(console_handle: *mut c_void, mode: *mut u32) -> i32;
    }

    #[link(name = "ucrt")]
    unsafe extern "C" {
        fn _getch() -> i32;
    }

    pub(crate) fn read_line(
        runtime: &mut Runtime,
        prompt: &str,
    ) -> Result<Option<ConsoleRead>, String> {
        if !stdin_is_console() {
            return Ok(None);
        }

        let mut line = String::new();
        loop {
            // SAFETY: `_getch` is the UCRT console reader and requires no
            // caller-owned pointer. This branch is entered only for a console.
            let character = unsafe { _getch() };
            match character {
                0 | 224 => {
                    // SAFETY: consume the second byte of an extended key.
                    let _ = unsafe { _getch() };
                }
                10 | 13 => {
                    super::super::write_stdout_text(b"\n").map_err(|error| error.to_string())?;
                    return Ok(Some(ConsoleRead::Line(line)));
                }
                9 => complete(runtime, prompt, &mut line)?,
                8 | 127 => {
                    if line.pop().is_some() {
                        super::super::write_stdout_text(b"\x08 \x08")
                            .map_err(|error| error.to_string())?;
                    }
                }
                3 => {
                    super::super::write_stdout_text(b"\n").map_err(|error| error.to_string())?;
                    return Ok(Some(ConsoleRead::Interrupted));
                }
                4 | 26 => return Ok(Some(ConsoleRead::Eof)),
                printable if (32..=126).contains(&printable) => {
                    let byte = printable as u8;
                    line.push(char::from(byte));
                    super::super::write_stdout_text(&[byte]).map_err(|error| error.to_string())?;
                }
                _ => {}
            }
        }
    }

    fn complete(runtime: &mut Runtime, prompt: &str, line: &mut String) -> Result<(), String> {
        let completion = super::super::repl_completion::complete(runtime, line)?;
        if completion.candidates.is_empty() {
            super::super::write_stdout_text(b"\x07").map_err(|error| error.to_string())?;
            return Ok(());
        }

        let old_line = line.clone();
        *line = completion.line;
        if completion.candidates.len() > 1 {
            let output = format!("\n{}\n{prompt}{line}", completion.candidates.join("  "));
            super::super::write_stdout_text(output.as_bytes())
                .map_err(|error| error.to_string())?;
        } else if let Some(suffix) = line.strip_prefix(&old_line) {
            super::super::write_stdout_text(suffix.as_bytes())
                .map_err(|error| error.to_string())?;
        } else {
            let output = format!("\n{prompt}{line}");
            super::super::write_stdout_text(output.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn stdin_is_console() -> bool {
        // SAFETY: both calls receive the process standard-input handle and a
        // valid local out pointer. INVALID_HANDLE_VALUE/null both fail closed.
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            if handle.is_null() || handle as isize == -1 {
                return false;
            }
            let mut mode = 0;
            GetConsoleMode(handle, &mut mode) != 0
        }
    }
}

#[cfg(windows)]
pub(crate) use platform::read_line;

#[cfg(not(windows))]
pub(crate) fn read_line(
    _runtime: &mut Runtime,
    _prompt: &str,
) -> Result<Option<ConsoleRead>, String> {
    Ok(None)
}
