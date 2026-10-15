//! 真实 Vm/library/binary 对照，不修改生产 trace 或 gc_roots。
use crate::{model, process};
use lua_compiler::compile;
use lua_core::{ByteString, HeapError, Instruction, Opcode, Proto, Value};
use lua_vm::{Vm, VmError, VmTraceEvent};
use std::path::Path;

fn require(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

pub fn arithmetic_events() -> Result<(Vec<Value>, Vec<VmTraceEvent>), String> {
    let mut vm = Vm::new();
    vm.enable_trace();
    let result = vm
        .execute_multi(&model::arithmetic_proto())
        .map_err(|error| error.to_string())?;
    Ok((result, vm.take_trace()))
}

pub fn branch_events() -> Result<(Vec<Value>, Vec<VmTraceEvent>), String> {
    let mut proto = model::arithmetic_proto();
    proto.constants = vec![
        Value::Boolean(false),
        Value::Number(999.0),
        Value::Number(42.0),
    ];
    proto.code = vec![
        Instruction::abx(Opcode::LoadK, 0, 0),
        Instruction::asbx(Opcode::JumpIfFalse, 0, 1),
        Instruction::abx(Opcode::LoadK, 1, 1), // 跳过 pc=2。
        Instruction::abx(Opcode::LoadK, 1, 2),
        Instruction::abc(Opcode::Return, 1, 2, 0),
    ];
    proto.lines = vec![20, 21, 22, 23, 24];
    proto.debug_name = "trace-branch".into();
    let mut vm = Vm::new();
    vm.enable_trace();
    let result = vm
        .execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    Ok((result, vm.take_trace()))
}

pub fn control_counts() -> Result<[usize; 5], String> {
    let proto = model::arithmetic_proto();
    let mut vm = Vm::new();
    vm.execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    let default_off = vm.take_trace().len();
    vm.enable_trace();
    vm.execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    vm.execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    let accumulated = vm.take_trace().len();
    vm.execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    let disabled_by_take = vm.take_trace().len();
    vm.enable_trace();
    vm.execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    vm.enable_trace(); // 清掉上次四项，而不是追加。
    vm.execute_multi(&proto)
        .map_err(|error| error.to_string())?;
    let cleared_by_enable = vm.take_trace().len();
    let moved_out = vm.take_trace().len();
    Ok([
        default_off,
        accumulated,
        disabled_by_take,
        cleared_by_enable,
        moved_out,
    ])
}

fn failing(proto: &Proto, limit: usize) -> Result<(VmError, Vec<VmTraceEvent>), String> {
    let mut vm = Vm::with_instruction_limit(limit);
    vm.enable_trace();
    match vm.execute_multi(proto) {
        Err(error) => Ok((error, vm.take_trace())),
        Ok(_) => Err("expected this malformed/limited fixture to fail".into()),
    }
}

pub struct FailureReport {
    pub failed_load: VmError,
    pub load_events: Vec<VmTraceEvent>,
    pub invalid: VmError,
    pub invalid_events: Vec<VmTraceEvent>,
    pub budget: VmError,
    pub budget_events: Vec<VmTraceEvent>,
    pub zero_budget_events: usize,
}

// ANCHOR: bytecode_trace_failure_boundaries
pub fn failure_boundaries() -> Result<FailureReport, String> {
    let mut bad_constant = model::arithmetic_proto();
    bad_constant.code[0] = Instruction::abx(Opcode::LoadK, 0, 99);
    let (failed_load, load_events) = failing(&bad_constant, 100)?;
    let mut invalid_opcode = model::arithmetic_proto();
    invalid_opcode.code[0] = Instruction::from_raw(63);
    let (invalid, invalid_events) = failing(&invalid_opcode, 100)?;
    let (budget, budget_events) = failing(&model::arithmetic_proto(), 2)?;
    let (_, zero_budget_events) = failing(&model::arithmetic_proto(), 0)?;
    Ok(FailureReport {
        failed_load,
        load_events,
        invalid,
        invalid_events,
        budget,
        budget_events,
        zero_budget_events: zero_budget_events.len(),
    })
}
// ANCHOR_END: bytecode_trace_failure_boundaries

#[derive(Debug, PartialEq)]
pub struct IdentityReport {
    pub earlier_handle_found: bool,
    pub current_field: Value,
    pub internal_trace_contains_reclaimed_handle: bool,
    pub republish_rejected_as_stale: bool,
}

fn execute(vm: &mut Vm, source: &[u8]) -> Result<Value, String> {
    let proto = compile(source).map_err(|error| error.to_string())?;
    vm.execute(&proto).map_err(|error| error.to_string())
}

pub fn identity_is_not_deep_snapshot_or_gc_root() -> Result<IdentityReport, String> {
    let mut vm = Vm::new();
    vm.enable_trace();
    let target = execute(
        &mut vm,
        b"trace_object={answer=40};trace_object.answer=42;return trace_object",
    )?;
    let history = vm.take_trace();
    let earlier = history
        .iter()
        .flat_map(|event| &event.registers)
        .find(|value| **value == target)
        .cloned()
        .ok_or("no table handle in real trace")?;
    let key = Value::String(ByteString::new(&b"trace_history"[..]));
    vm.registry_set(key, earlier)
        .map_err(|error| error.to_string())?;
    let current_field = execute(&mut vm, b"return debug.getregistry().trace_history.answer")?;
    require(
        current_field == Value::Number(42.0),
        "earlier identity must read current field 42",
    )?;

    vm.enable_trace();
    let transient = execute(&mut vm, b"transient={answer=13};return transient")?;
    execute(&mut vm, b"transient=nil;return true")?; // 结束旧结果 roots，并移除 global。
    vm.collect_garbage().map_err(|error| error.to_string())?;
    // 此刻 trace_events 仍在 Vm 里面，但它不在 gc_roots 中。
    let republish_rejected_as_stale = matches!(
        vm.registry_set(
            Value::String(ByteString::new(&b"must_not_revive"[..])),
            transient.clone()
        ),
        Err(HeapError::StaleHandle { .. })
    );
    let internal_trace_contains_reclaimed_handle = vm
        .take_trace()
        .iter()
        .any(|event| event.registers.contains(&transient));
    require(
        republish_rejected_as_stale,
        "internal trace must not root the transient table",
    )?;
    require(
        internal_trace_contains_reclaimed_handle,
        "expected reclaimed typed identity in trace",
    )?;
    Ok(IdentityReport {
        earlier_handle_found: true,
        current_field,
        internal_trace_contains_reclaimed_handle,
        republish_rejected_as_stale,
    })
}

// 独立字面期望：编译 return 40+2 会额外 Move 到返回窗口，不能沿用四条手写 Proto。
pub const PURE_TRACE_JSON: &str = r#"[
  {"step":0,"depth":0,"function":"chunk","pc":0,"line":1,"op":"LoadK","registers":[{"type":"nil"},{"type":"nil"},{"type":"nil"}]},
  {"step":1,"depth":0,"function":"chunk","pc":1,"line":1,"op":"LoadK","registers":[{"type":"number","value":40},{"type":"nil"},{"type":"nil"}]},
  {"step":2,"depth":0,"function":"chunk","pc":2,"line":1,"op":"Add","registers":[{"type":"number","value":40},{"type":"number","value":2},{"type":"nil"}]},
  {"step":3,"depth":0,"function":"chunk","pc":3,"line":1,"op":"Move","registers":[{"type":"number","value":42},{"type":"number","value":2},{"type":"nil"}]},
  {"step":4,"depth":0,"function":"chunk","pc":4,"line":1,"op":"Return","registers":[{"type":"number","value":42},{"type":"number","value":2},{"type":"number","value":42}]}
]
"#;

#[derive(Debug, PartialEq, Eq)]
pub struct BinaryReport {
    pub pure_events: usize,
    pub runtime_exit: i32,
    pub runtime_stdout_bytes: usize,
    pub partial_library_events: usize,
    pub print_pollutes_stdout: bool,
}

pub fn binary_cases(binary: &Path) -> Result<BinaryReport, String> {
    let args = vec!["--format".into(), "trace".into(), "-".into()];
    let pure = process::run(binary, &args, b"return 40+2\n")?;
    require(
        pure.status.code() == Some(0) && pure.stderr.is_empty(),
        "pure trace process failed",
    )?;
    require(
        pure.stdout == PURE_TRACE_JSON.as_bytes(),
        "pure process trace differs from independent five-event JSON",
    )?;

    let error_source = b"local x=40\nerror('trace-stop')\n";
    let failed = process::run(binary, &args, error_source)?;
    require(
        failed.status.code() == Some(1),
        "runtime error must exit one",
    )?;
    require(
        failed.stdout.is_empty(),
        "CLI error path must not render partial trace",
    )?;
    require(
        String::from_utf8_lossy(&failed.stderr).contains("trace-stop"),
        "missing runtime diagnostic",
    )?;
    let mut vm = Vm::new();
    vm.enable_trace();
    require(
        execute(&mut vm, error_source).is_err(),
        "library must report the same failure",
    )?;
    let partial = vm.take_trace();
    require(
        partial
            .last()
            .is_some_and(|event| event.opcode == Opcode::Call),
        "library must retain failing Call's before event",
    )?;

    let printed = process::run(binary, &args, b"print('script-output');return 42\n")?;
    require(
        printed.status.code() == Some(0) && printed.stderr.is_empty(),
        "print fixture failed",
    )?;
    let print_pollutes_stdout =
        printed.stdout.starts_with(b"script-output\n[\n") && printed.stdout.ends_with(b"\n]\n");
    require(
        print_pollutes_stdout,
        "Lua print should precede JSON on the same stdout",
    )?;
    Ok(BinaryReport {
        pure_events: 5,
        runtime_exit: 1,
        runtime_stdout_bytes: failed.stdout.len(),
        partial_library_events: partial.len(),
        print_pollutes_stdout,
    })
}
