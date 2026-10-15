use crate::chunks::{ADVANCE, SETUP};
use crate::contracts::equal;
use crate::{process, temp};
use lua_compiler::compile;
use lua_core::{ByteString, HeapError, Value};
use lua_vm::Vm;
use std::fs;
use std::path::Path;

fn run(vm: &mut Vm, source: &[u8]) -> Result<Value, String> {
    let proto = compile(source).map_err(|error| error.to_string())?;
    vm.execute(&proto).map_err(|error| error.to_string())
}

#[derive(Debug, PartialEq)]
pub struct HostRootTrace {
    pub unrooted_insert_is_stale: bool,
    pub registry_identity: bool,
    pub registry_field: Value,
    pub removed_slot: Value,
    pub released_insert_is_stale: bool,
}

// Public Vm probe: Session deliberately does not expose its private Vm.
pub fn copied_host_handle_is_not_a_root() -> Result<HostRootTrace, String> {
    let mut vm = Vm::new();
    let key = Value::String(ByteString::new(&b"repl_owner_host"[..]));
    let unrooted = run(&mut vm, b"return { answer = 13 }")?;
    run(&mut vm, b"return true")?; // Clear the previous chunk's temporary result roots.
    vm.collect_garbage().map_err(|error| error.to_string())?;
    let unrooted_insert_is_stale = matches!(
        vm.registry_set(key.clone(), unrooted.clone()),
        Err(HeapError::StaleHandle { .. })
    );
    equal(
        "host copy alone is rejected as stale",
        unrooted_insert_is_stale,
        true,
    )?;
    let kept = run(&mut vm, b"return { answer = 42 }")?;
    vm.registry_set(key.clone(), kept.clone())
        .map_err(|error| error.to_string())?;
    run(&mut vm, b"return true")?;
    vm.collect_garbage().map_err(|error| error.to_string())?;
    let registry_identity = vm.registry_get(&key).map_err(|error| error.to_string())? == kept;
    let registry_field = run(
        &mut vm,
        b"return debug.getregistry().repl_owner_host.answer",
    )?;
    equal("registry handle identity", registry_identity, true)?;
    equal(
        "actual GC-kept table access",
        registry_field.clone(),
        Value::Number(42.0),
    )?;
    vm.registry_set(key.clone(), Value::Nil)
        .map_err(|error| error.to_string())?;
    run(&mut vm, b"return true")?;
    vm.collect_garbage().map_err(|error| error.to_string())?;
    let removed_slot = vm.registry_get(&key).map_err(|error| error.to_string())?;
    let released_insert_is_stale = matches!(
        vm.set_global(b"must_not_republish", kept.clone()),
        Err(HeapError::StaleHandle { .. })
    );
    equal(
        "registry key is exactly nil",
        removed_slot.clone(),
        Value::Nil,
    )?;
    equal(
        "still-owned Rust copy is stale after removing root",
        released_insert_is_stale,
        true,
    )?;
    Ok(HostRootTrace {
        unrooted_insert_is_stale,
        registry_identity,
        registry_field,
        removed_slot,
        released_insert_is_stale,
    })
}

fn text(bytes: Vec<u8>, label: &str) -> Result<String, String> {
    String::from_utf8(bytes)
        .map(|text| text.replace("\r\n", "\n"))
        .map_err(|error| format!("{label} is not UTF-8: {error}"))
}

fn successful_output(output: std::process::Output) -> Result<String, String> {
    equal("child exit code", output.status.code(), Some(0))?;
    equal(
        "child stderr",
        text(output.stderr, "stderr")?,
        String::new(),
    )?;
    text(output.stdout, "stdout")
}

// ANCHOR: repl_owner_cli_script
const SCRIPT: &[u8] = br#"
gc_collect()
local module = require('repl_owner_mod')
assert(chunk_only == nil and module == package.loaded.repl_owner_mod)
print('script', repl_number, repl_table.answer, repl_next(), module.answer,
  repl_loads, debug.getregistry().repl_owner_cell.answer)
"#;
// ANCHOR_END: repl_owner_cli_script

pub fn cli_multiple_actions_then_script(binary: &Path) -> Result<String, String> {
    let directory = temp::TempDirectory::new("repl-owner-script")?;
    let result = (|| {
        let script = directory.path("state.lua");
        fs::write(&script, SCRIPT).map_err(|error| error.to_string())?;
        let arguments = vec![
            "-e".to_owned(),
            String::from_utf8(SETUP.to_vec()).unwrap(),
            "-e".to_owned(),
            String::from_utf8(ADVANCE.to_vec()).unwrap(),
            script
                .to_str()
                .ok_or("script path must be UTF-8")?
                .to_owned(),
        ];
        let output = successful_output(process::run(binary, &arguments, b"")?)?;
        equal(
            "multiple -e and script share one Session",
            output.clone(),
            "script\t11\t42\t5\t8\t1\t10\n".to_owned(),
        )?;
        Ok(output)
    })();
    directory.finish(result)
}

// ANCHOR: repl_owner_cli_repl
const INPUTS: &[u8] = br#"return repl_number, repl_table.answer, repl_next(), require('repl_owner_mod').answer, debug.getregistry().repl_owner_cell.answer, chunk_only
repl_table.answer = 42; repl_number = 11; return true
gc_collect(); return repl_number, repl_table.answer, repl_next(), require('repl_owner_mod') == package.loaded.repl_owner_mod, repl_loads
"#;
// ANCHOR_END: repl_owner_cli_repl

pub fn cli_actions_then_interactive_inputs(binary: &Path) -> Result<String, String> {
    let arguments = vec![
        "-e".to_owned(),
        String::from_utf8(SETUP.to_vec()).unwrap(),
        "-i".to_owned(),
    ];
    let output = successful_output(process::run(binary, &arguments, INPUTS)?)?;
    equal(
        "-e and three REPL inputs share one Session",
        output.clone(),
        concat!(
            "Lua 5.1 (lua-rust teaching v0.18)\n",
            "> 10\t40\t4\t7\t9\tnil\n> true\n> 11\t42\t5\ttrue\t1\n> \n"
        )
        .to_owned(),
    )?;
    Ok(output)
}
