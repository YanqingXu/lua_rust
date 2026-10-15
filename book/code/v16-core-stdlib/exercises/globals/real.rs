use lua_compiler::compile;
use lua_core::{ByteString, Heap, Value};
use lua_vm::Vm;

fn run(vm: &mut Vm, source: &[u8]) -> Value {
    vm.execute(&compile(source).unwrap()).unwrap()
}

pub fn heap_self_referential_environment_needs_a_root() -> [bool; 3] {
    let mut heap = Heap::new();
    let owner = heap.allocate_table().unwrap();
    let child = heap.allocate_table().unwrap();
    let key = |name: &str| Value::String(ByteString::new(name.as_bytes()));
    heap.table_set(owner, key("_G"), Value::Table(owner))
        .unwrap();
    heap.table_set(owner, key("child"), Value::Table(child))
        .unwrap();
    heap.table_set(child, key("answer"), Value::Number(42.0))
        .unwrap();
    let copied_owner = owner;
    heap.collect(&[Value::Table(owner)], &[]).unwrap();
    let kept = heap.table_get(owner, &key("_G")) == Ok(Value::Table(owner))
        && heap.table_get(child, &key("answer")) == Ok(Value::Number(42.0));
    heap.collect(&[], &[]).unwrap();
    [
        kept,
        heap.is_live(copied_owner) == Ok(false)
            && heap.table_get(copied_owner, &key("_G")).is_err(),
        heap.is_live(child) == Ok(false) && heap.table_get(child, &key("answer")).is_err(),
    ]
}

// ANCHOR: globals_real_vm_bidirectional
pub const BIDIRECTIONAL: &[u8] = br#"
forward = 12
local a = _G.forward == 12
_G.reverse = 34
local b = reverse == 34
forward = nil
local c = forward == nil and rawget(_G, 'forward') == nil
_G.reverse = nil
local d = reverse == nil and rawget(_G, 'reverse') == nil
return a and b and c and d and _G._G == _G
"#;
// ANCHOR_END: globals_real_vm_bidirectional

pub fn vm_bidirectional() -> Value {
    run(&mut Vm::new(), BIDIRECTIONAL)
}

// ANCHOR: globals_real_vm_gc
pub fn vm_reachable_graph_across_execute_and_gc() -> [Value; 3] {
    let mut vm = Vm::new();
    let installed = run(
        &mut vm,
        br#"
      weak = setmetatable({}, { __mode = 'v' })
      shared = { child = { answer = 42 } }
      shared.self = shared
      weak.owner = shared
      weak.child = shared.child
      return _G.shared.child.answer == 42
    "#,
    );
    vm.collect_garbage().unwrap();
    let kept = run(
        &mut vm,
        br#"
      return shared == _G.shared and _G.shared.child.answer == 42
        and weak.owner == shared and weak.child == shared.child
    "#,
    );
    assert_eq!(
        run(&mut vm, b"shared = nil; return true"),
        Value::Boolean(true)
    );
    vm.collect_garbage().unwrap();
    let released = run(
        &mut vm,
        br#"
      return weak.owner == nil and weak.child == nil and shared == nil and _G._G == _G
    "#,
    );
    [installed, kept, released]
}
// ANCHOR_END: globals_real_vm_gc

pub fn vm_rebinding_g_preserves_environment() -> [Value; 3] {
    let mut vm = Vm::new();
    let replaced = run(
        &mut vm,
        br#"
      original = _G
      _G = { sentinel = 5 }
      bare = 17
      _G.field = 23
      return original ~= _G and original._G == _G and original.bare == 17
        and original.field == nil and field == nil and _G.field == 23
    "#,
    );
    vm.collect_garbage().unwrap();
    let removed = run(
        &mut vm,
        br#"
      _G = nil
      bare = 29
      return original.bare == 29 and original._G == nil and _G == nil
    "#,
    );
    assert_eq!(
        run(&mut vm, b"original = nil; return true"),
        Value::Boolean(true)
    );
    vm.collect_garbage().unwrap();
    let still_usable = run(
        &mut vm,
        br#"
      bare = 31
      return bare == 31 and _G == nil and original == nil and math.abs(-4) == 4
    "#,
    );
    [replaced, removed, still_usable]
}
