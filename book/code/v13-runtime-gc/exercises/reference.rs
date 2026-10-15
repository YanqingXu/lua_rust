mod contracts;
#[path = "reference/solution.rs"]
mod solution;

use lua_compiler::compile;
use lua_core::{Heap, HeapError, Instruction, Opcode, Proto, StackSlot, Value};
use lua_vm::Vm;
use std::cell::RefCell;
use std::rc::Rc;

#[test]
fn two_table_cycle_loses_its_last_global_root() {
    contracts::two_table_cycle(include_str!("reference/cycle.lua"));
}

#[test]
fn host_registry_lifetime() {
    contracts::host_registry_lifetime(solution::pin, solution::unpin);
}

#[test]
fn close_before_register_reuse() {
    contracts::close_before_register_reuse(solution::restore_scope_exit);
}

#[test]
fn primitive_metatable_root_lifetime() {
    contracts::primitive_metatable_root_lifetime(solution::primitive_roots);
}

#[test]
fn primitive_metatable_remains_usable_after_scope_and_collection() {
    let proto = compile(include_bytes!("primitive_metatable.lua")).unwrap();
    let mut vm = Vm::new();
    assert_eq!(vm.execute(&proto), Ok(Value::Number(42.0)));
}

#[test]
fn mark_failure_preserves_heap_and_clears_partial_marks() {
    let mut heap = Heap::new();
    let abandoned = heap.allocate_table().unwrap();
    let parent = heap.allocate_table().unwrap();
    let first_child = heap.allocate_table().unwrap();
    let unrelated = heap.allocate_table().unwrap();
    heap.table_set(parent, Value::Number(1.0), Value::Table(first_child))
        .unwrap();

    let slot: StackSlot = Rc::new(RefCell::new(Value::Number(7.0)));
    let upvalue = heap.allocate_open_upvalue(slot.clone()).unwrap();
    let proto = Rc::new(Proto {
        constants: vec![Value::Table(parent)],
        code: vec![Instruction::abc(Opcode::Return, 0, 1, 0)],
        lines: vec![1],
        max_stack_size: 1,
        protos: vec![],
        num_params: 0,
        is_vararg: false,
        upvalues: vec![],
        debug_name: "mark-failure".to_owned(),
    });
    let function = heap
        .allocate_function_with_upvalues(proto, vec![upvalue])
        .unwrap();

    // 当前工作栈先追踪 abandoned，再追踪 function 的 Proto 常量图，
    // 最后访问 Open upvalue：错误发生在多个 record 已经 mark 之后。
    let borrowed = slot.borrow_mut();
    assert_eq!(
        heap.collect(&[Value::Table(abandoned), Value::Function(function)], &[]),
        Err(HeapError::StackBorrowConflict)
    );
    assert_eq!(heap.live_count(), 6, "mark 失败时不能执行 sweep");
    assert_eq!(heap.is_live(unrelated), Ok(true));
    drop(borrowed);

    // 新增失败时尚不存在的 child，随后换 roots 重试。若残留旧 mark，
    // abandoned 会假存活，parent 也会跳过这条新边，使 later_child 被误收。
    let later_child = heap.allocate_table().unwrap();
    heap.table_set(parent, Value::Number(2.0), Value::Table(later_child))
        .unwrap();
    let retried = heap.collect(&[Value::Table(parent)], &[]).unwrap();
    assert_eq!((retried.collected, retried.remaining), (4, 3));
    assert_eq!(heap.is_live(abandoned), Ok(false));
    assert_eq!(heap.is_live(function), Ok(false));
    assert_eq!(heap.is_live(upvalue), Ok(false));
    assert_eq!(heap.is_live(first_child), Ok(true));
    assert_eq!(heap.is_live(later_child), Ok(true));
    assert_eq!(heap.collect(&[], &[]).unwrap().remaining, 0);
}
