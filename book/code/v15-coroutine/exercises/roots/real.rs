use lua_compiler::compile;
use lua_core::{ByteString, CoroutineRef, FunctionRef, Heap, TableRef, UpvalueRef, Value};
use lua_vm::Vm;
use std::cell::RefCell;
use std::rc::Rc;

fn key(name: &str) -> Value {
    Value::String(ByteString::new(name.as_bytes()))
}

struct Fixture {
    heap: Heap,
    prefix: TableRef,
    thread: CoroutineRef,
    function: FunctionRef,
    getter: FunctionRef,
    open: UpvalueRef,
    closed: UpvalueRef,
    cycle: TableRef,
    captured: TableRef,
    register: TableRef,
    vararg: TableRef,
    register_slot: Rc<RefCell<Value>>,
    cycle_slot: Rc<RefCell<Value>>,
}

impl Fixture {
    fn new(mirror: bool) -> Self {
        let mut heap = Heap::new();
        let prefix = heap.allocate_table().unwrap();
        let thread = heap.allocate_coroutine(Vec::new()).unwrap();
        let captured = heap.allocate_table().unwrap();
        let cycle = heap.allocate_table().unwrap();
        let register = heap.allocate_table().unwrap();
        let vararg = heap.allocate_table().unwrap();
        for table in [prefix, captured, cycle, register, vararg] {
            heap.table_set(table, key("answer"), Value::Number(42.0))
                .unwrap();
        }
        heap.table_set(cycle, key("thread"), Value::Coroutine(thread))
            .unwrap();
        let cycle_slot = Rc::new(RefCell::new(Value::Table(cycle)));
        let open = heap.allocate_open_upvalue(Rc::clone(&cycle_slot)).unwrap();
        let closed_slot = Rc::new(RefCell::new(Value::Table(captured)));
        let closed = heap.allocate_open_upvalue(closed_slot).unwrap();
        heap.close_upvalue(closed).unwrap();
        let outer = compile(b"local value; return function() return value end").unwrap();
        let function = heap
            .allocate_function_with_upvalues(Rc::clone(&outer.protos[0]), vec![closed])
            .unwrap();
        let getter = heap
            .allocate_function_with_upvalues(Rc::clone(&outer.protos[0]), vec![open])
            .unwrap();
        let register_slot = Rc::new(RefCell::new(Value::Table(register)));
        if mirror {
            heap.coroutine_set_roots(
                thread,
                vec![
                    Value::Function(function),
                    Value::Table(vararg),
                    register_slot.borrow().clone(),
                    Value::Function(getter),
                ],
            )
            .unwrap();
        }
        Self {
            heap,
            prefix,
            thread,
            function,
            getter,
            open,
            closed,
            cycle,
            captured,
            register,
            vararg,
            register_slot,
            cycle_slot,
        }
    }

    fn observe(&self) -> HeapObservation {
        // 两个执行 slot 的 Rc 仍由 Rust 持有；下面另行实际访问目标，不能用句柄相等代替存活。
        assert_eq!(*self.register_slot.borrow(), Value::Table(self.register));
        assert_eq!(*self.cycle_slot.borrow(), Value::Table(self.cycle));
        let table_live =
            |table| self.heap.table_get(table, &key("answer")) == Ok(Value::Number(42.0));
        HeapObservation {
            thread: self.heap.is_live(self.thread) == Ok(true),
            graph: [
                self.heap.function_proto(self.function).is_ok(),
                self.heap.function_proto(self.getter).is_ok(),
                self.heap.upvalue_get(self.open) == Ok(Value::Table(self.cycle)),
                table_live(self.cycle),
                self.heap.upvalue_get(self.closed) == Ok(Value::Table(self.captured)),
                table_live(self.captured),
                table_live(self.register),
                table_live(self.vararg),
            ],
            prefix: table_live(self.prefix),
        }
    }
}

#[derive(Debug)]
pub struct HeapObservation {
    pub thread: bool,
    pub graph: [bool; 8],
    pub prefix: bool,
}

pub fn heap_mirror_then_release(mirror: bool) -> [HeapObservation; 2] {
    let mut fixture = Fixture::new(mirror);
    fixture
        .heap
        .collect(
            &[
                Value::Table(fixture.prefix),
                Value::Coroutine(fixture.thread),
            ],
            &[],
        )
        .unwrap();
    let kept = fixture.observe();
    fixture
        .heap
        .collect(&[Value::Table(fixture.prefix)], &[])
        .unwrap();
    assert_eq!(fixture.heap.is_live(fixture.thread), Ok(false));
    [kept, fixture.observe()]
}

// ANCHOR: coroutine_roots_real_vm
pub const VM_CREATE: &[u8] = br#"
local captured = { delta = 1 }
co = coroutine.create(function(...)
  local cycle = { answer = 40 }
  cycle.thread = coroutine.running()
  local getter = function() return cycle end
  local saved = { answer = 2 }
  coroutine.yield(1)
  local payload = ...
  return captured.delta + getter().answer + saved.answer + payload.delta
end)
return co ~= nil
"#;
// ANCHOR_END: coroutine_roots_real_vm

fn run(vm: &mut Vm, source: &[u8]) -> Value {
    vm.execute(&compile(source).unwrap()).unwrap()
}

pub fn vm_keeps_entry_and_suspended_values() -> Value {
    let mut vm = Vm::new();
    assert_eq!(run(&mut vm, VM_CREATE), Value::Boolean(true));
    vm.collect_garbage().unwrap(); // 首次 resume 前：只能由 thread 的 entry 镜像保活。
    assert_eq!(
        run(
            &mut vm,
            br#"
      local ok, yielded = coroutine.resume(co, { delta = 3 })
      return ok and yielded == 1
    "#
        ),
        Value::Boolean(true)
    );
    vm.collect_garbage().unwrap(); // 上个主执行帧已结束，暂停 varargs 是 payload 唯一执行路径。
    run(
        &mut vm,
        br#"
      local ok, value = coroutine.resume(co)
      return ok and value == 46 and coroutine.status(co) == 'dead'
    "#,
    )
}

pub fn vm_unreachable_cycle_is_pruned() -> (usize, bool, Value) {
    let mut vm = Vm::new();
    assert_eq!(
        run(
            &mut vm,
            br#"
      prefix = { answer = 7 }
      weak = setmetatable({}, { __mode = 'v' })
      co = coroutine.create(function()
        local cycle = {}
        cycle.self = cycle
        cycle.thread = coroutine.running()
        local getter = function() return cycle end
        weak.cycle = cycle
        weak.getter = getter
        weak.thread = coroutine.running()
        coroutine.yield()
        return getter()
      end)
      local ok = coroutine.resume(co)
      return ok
    "#
        ),
        Value::Boolean(true)
    );
    assert_eq!(vm.coroutine_state_count(), 1);
    assert_eq!(run(&mut vm, b"co = nil; return true"), Value::Boolean(true));
    let before = vm.heap_live_count();
    vm.collect_garbage().unwrap();
    let decreased = vm.heap_live_count() < before;
    let states = vm.coroutine_state_count();
    let slots = run(
        &mut vm,
        br#"
      return weak.cycle == nil and weak.getter == nil and weak.thread == nil
        and prefix.answer == 7
    "#,
    );
    (states, decreased, slots)
}
