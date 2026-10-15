//! The real compiler owns the syntax decision; the REPL only chooses its next action.
use lua_compiler::{compile, CompileError};
use lua_core::{Proto, Value};
use lua_vm::Vm;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Decision {
    Execute,
    Preserve,
    Reject,
}

pub type Decide = fn(&Result<Proto, CompileError>) -> Decision;

pub fn complete(source: &[u8]) -> Result<Vec<Value>, String> {
    let proto = compile(source).map_err(|error| error.to_string())?;
    Vm::new()
        .execute_multi(&proto)
        .map_err(|error| error.to_string())
}

/// Each suffix starts after a physical newline, as it would in the REPL buffer.
pub fn unfinished() -> Vec<(&'static str, &'static [u8], &'static [u8])> {
    vec![
        ("expression", b"return 40 +\n", b"2\n"),
        ("if block", b"if true then\nreturn 42\n", b"end\n"),
        ("parenthesis", b"return (40 +\n", b"2)\n"),
        ("table", b"local t={answer=40+\n", b"2}; return t.answer\n"),
        (
            "function",
            b"local f=function()\nreturn 42\n",
            b"end; return f()\n",
        ),
        ("long string", b"return [=[hello\n", b"world]=]\n"),
        ("long comment", b"--[=[hello\n", b"world]=]\nreturn 42\n"),
    ]
}

pub fn syntax_errors() -> Vec<(&'static str, &'static [u8])> {
    vec![
        ("illegal token", b"return @\n"),
        ("wrong variable", b"local =\n"),
        ("short string newline", b"return 'hello\n"),
        ("exponent at EOF", b"return 1e"),
        ("hex at EOF", b"return 0x"),
        ("decimal escape at EOF", b"return '\\256"),
    ]
}
