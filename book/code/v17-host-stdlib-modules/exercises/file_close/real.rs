//! Unchanged VM probes: observable aliases/errors, real GC edges and OS sharing.
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::{Vm, VmErrorKind};
use std::path::Path;

use super::fixture::{exclusive_available, Fixture};

fn lua_quote(path: &Path) -> String {
    let text = path
        .to_str()
        .expect("fixture path must be UTF-8 on this snapshot");
    format!(
        "\"{}\"",
        text.bytes()
            .map(|byte| format!("\\{byte:03}"))
            .collect::<String>()
    )
}

fn run(vm: &mut Vm, source: &str) -> Value {
    vm.execute(&compile(source.as_bytes()).unwrap()).unwrap()
}

// ANCHOR: file_close_real_aliases
pub fn both_close_routes_share_alias_state() -> Value {
    let fixture = Fixture::new();
    let mut vm = Vm::new();
    let result = run(
        &mut vm,
        &format!(
            r#"
      local first=assert(io.open({},'w+'))
      local alias=first
      local before=first==alias and io.type(alias)=='file'
      local closed=io.close(first)
      local again=pcall(function() alias:close() end)
      local second=assert(io.open({},'w+'))
      local second_alias=second
      local closed_second=second_alias:close()
      local again_second=pcall(io.close,second)
      return before and closed and not again and io.type(alias)=='closed file'
        and second==second_alias and closed_second and not again_second
        and io.type(second)=='closed file' and io.type({{}})==nil
    "#,
            lua_quote(&fixture.path("first.dat")),
            lua_quote(&fixture.path("second.dat"))
        ),
    );
    drop(vm);
    fixture.finish();
    result
}
// ANCHOR_END: file_close_real_aliases

#[derive(Debug)]
pub struct ErrorProbe {
    pub operation: &'static str,
    pub kind: VmErrorKind,
    pub message: String,
}

pub fn closed_and_wrong_type_error_matrix() -> Vec<ErrorProbe> {
    let fixture = Fixture::new();
    let mut vm = Vm::new();
    assert_eq!(run(&mut vm,&format!("closed=assert(io.open({},'w+')); methods=debug.getmetatable(closed); closed:close(); return io.type(closed)=='closed file'",lua_quote(&fixture.path("errors.dat")))),Value::Boolean(true));
    let mut probes = Vec::new();
    for (operation, source, expected_kind, expected_message) in [
        (
            "io.close",
            "return io.close(closed)",
            VmErrorKind::Runtime,
            "attempt to close a closed file",
        ),
        (
            "file:close",
            "return closed:close()",
            VmErrorKind::Runtime,
            "attempt to close a closed file",
        ),
        (
            "file:read",
            "return closed:read('*a')",
            VmErrorKind::Runtime,
            "attempt to use a closed file",
        ),
        (
            "file:write",
            "return closed:write('x')",
            VmErrorKind::Runtime,
            "attempt to use a closed file",
        ),
        (
            "file:seek",
            "return closed:seek('set',0)",
            VmErrorKind::Runtime,
            "attempt to use a closed file",
        ),
        (
            "file:flush",
            "return closed:flush()",
            VmErrorKind::Runtime,
            "attempt to use a closed file",
        ),
        (
            "non-file io.close",
            "return io.close({})",
            VmErrorKind::Type,
            "expected userdata",
        ),
        (
            "non-file file:close",
            "return methods.close({})",
            VmErrorKind::Type,
            "expected userdata",
        ),
        (
            "non-file file:read",
            "return methods.read({})",
            VmErrorKind::Type,
            "expected userdata",
        ),
        (
            "non-file file:write",
            "return methods.write({},'x')",
            VmErrorKind::Type,
            "expected userdata",
        ),
        (
            "non-file file:seek",
            "return methods.seek({})",
            VmErrorKind::Type,
            "expected userdata",
        ),
        (
            "non-file file:flush",
            "return methods.flush({})",
            VmErrorKind::Type,
            "expected userdata",
        ),
    ] {
        let error = vm
            .execute(&compile(source.as_bytes()).unwrap())
            .unwrap_err();
        assert_eq!(error.kind, expected_kind, "{operation}");
        assert!(
            error.message.contains(expected_message),
            "{operation}: {}",
            error.message
        );
        probes.push(ErrorProbe {
            operation,
            kind: error.kind,
            message: error.message,
        });
    }
    drop(vm);
    fixture.finish();
    probes
}

// ANCHOR: file_close_real_metadata
pub fn closed_userdata_keeps_metatable_edge_after_gc() -> Value {
    let fixture = Fixture::new();
    let mut vm = Vm::new();
    assert_eq!(
        run(
            &mut vm,
            &format!(
                r#"
      closed=assert(io.open({},'w+'))
      local methods=debug.getmetatable(closed)
      debug.setmetatable(closed,{{__index=methods,metadata={{answer=42}}}})
      local alias=closed
      alias:close()
      return alias==closed and io.type(closed)=='closed file'
    "#,
                lua_quote(&fixture.path("metadata.dat"))
            )
        ),
        Value::Boolean(true)
    );
    vm.collect_garbage().unwrap();
    let result = run(
        &mut vm,
        r#"
      return io.type(closed)=='closed file'
        and debug.getmetatable(closed).metadata.answer==42
    "#,
    );
    drop(vm);
    fixture.finish();
    result
}
// ANCHOR_END: file_close_real_metadata

pub fn exclusive_reopen_before_and_after_close() -> (Option<bool>, Option<bool>) {
    let fixture = Fixture::new();
    let path = fixture.path("exclusive.dat");
    let mut vm = Vm::new();
    assert_eq!(
        run(
            &mut vm,
            &format!(
                "file=assert(io.open({},'w+')); return io.type(file)=='file'",
                lua_quote(&path)
            )
        ),
        Value::Boolean(true)
    );
    let before = exclusive_available(&path);
    assert_eq!(
        run(
            &mut vm,
            "local alias=file; io.close(alias); return io.type(file)=='closed file'"
        ),
        Value::Boolean(true)
    );
    let after = exclusive_available(&path);
    if before.is_some() {
        assert_eq!((before, after), (Some(false), Some(true)));
    }
    drop(vm);
    fixture.finish();
    (before, after)
}
