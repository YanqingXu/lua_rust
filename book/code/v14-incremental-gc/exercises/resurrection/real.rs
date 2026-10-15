//! 真实 v0.14 的可控 roots 对照；不修改 finalized、不执行重新注册 finalizer。
use lua_compiler::compile;
use lua_core::{ByteString, FunctionRef, Heap, TableRef, UpvalueRef, UserdataRef, Value};
use lua_vm::Vm;
use std::cell::RefCell;
use std::rc::Rc;

fn key(name: &str) -> Value {
    Value::String(ByteString::new(name.as_bytes()))
}

struct Fixture {
    heap: Heap,
    prefix: TableRef,
    weak: TableRef,
    userdata: UserdataRef,
    callback: FunctionRef,
    upvalue: UpvalueRef,
    child: TableRef,
    grandchild: TableRef,
}

impl Fixture {
    fn new() -> Self {
        let mut heap = Heap::new();
        let prefix = heap.allocate_table().unwrap();
        heap.table_set(prefix, key("answer"), Value::Number(7.0))
            .unwrap();
        let mode = heap.allocate_table().unwrap();
        heap.table_set(mode, key("__mode"), key("v")).unwrap();
        let weak = heap.allocate_table().unwrap();
        heap.table_set_metatable(weak, Some(mode)).unwrap();
        let userdata = heap.allocate_userdata().unwrap();
        let child = heap.allocate_table().unwrap();
        let grandchild = heap.allocate_table().unwrap();
        heap.table_set(grandchild, key("answer"), Value::Number(42.0))
            .unwrap();
        heap.table_set(child, key("next"), Value::Table(grandchild))
            .unwrap();
        let slot = Rc::new(RefCell::new(Value::Table(child)));
        let upvalue = heap.allocate_open_upvalue(Rc::clone(&slot)).unwrap();
        heap.close_upvalue(upvalue).unwrap();
        drop(slot);
        let outer = compile(b"local child; return function() return child end").unwrap();
        let callback = heap
            .allocate_function_with_upvalues(Rc::clone(&outer.protos[0]), vec![upvalue])
            .unwrap();
        heap.userdata_set_finalizer(userdata, Some(Value::Function(callback)))
            .unwrap();
        for (name, value) in [
            ("userdata", Value::Userdata(userdata)),
            ("callback", Value::Function(callback)),
            ("child", Value::Table(child)),
            ("grandchild", Value::Table(grandchild)),
        ] {
            heap.table_set(weak, key(name), value).unwrap();
        }
        Self {
            heap,
            prefix,
            weak,
            userdata,
            callback,
            upvalue,
            child,
            grandchild,
        }
    }

    fn observe(&self, total_events: usize) -> RealCycle {
        let expected = [
            ("userdata", Value::Userdata(self.userdata)),
            ("callback", Value::Function(self.callback)),
            ("child", Value::Table(self.child)),
            ("grandchild", Value::Table(self.grandchild)),
        ];
        let finalized = self.heap.userdata_is_finalized(self.userdata).ok();
        RealCycle {
            weak: expected.each_ref().map(|(name, value)| {
                self.heap.table_get(self.weak, &key(name)) == Ok(value.clone())
            }),
            weak_nil: expected
                .map(|(name, _)| self.heap.table_get(self.weak, &key(name)) == Ok(Value::Nil)),
            accessible: [
                finalized.is_some(),
                self.heap.function_proto(self.callback).is_ok(),
                self.heap.upvalue_get(self.upvalue) == Ok(Value::Table(self.child)),
                self.heap.table_get(self.child, &key("next")) == Ok(Value::Table(self.grandchild)),
                self.heap.table_get(self.grandchild, &key("answer")) == Ok(Value::Number(42.0)),
            ],
            prefix_live: self.heap.table_get(self.prefix, &key("answer")) == Ok(Value::Number(7.0)),
            finalized,
            total_events,
        }
    }
}

#[derive(Debug)]
pub struct RealCycle {
    pub weak: [bool; 4],
    pub weak_nil: [bool; 4],
    pub accessible: [bool; 5],
    pub prefix_live: bool,
    pub finalized: Option<bool>,
    pub total_events: usize,
}

/// core 不运行 Lua callback；这里模拟 VM 在首批事件后把 userdata 发布成长期 root。
pub fn heap_cycles(rescue: bool) -> Vec<RealCycle> {
    let mut fixture = Fixture::new();
    let mut roots = vec![Value::Table(fixture.prefix), Value::Table(fixture.weak)];
    let mut total_events = 0;
    let mut observations = Vec::new();
    for cycle in 0..if rescue { 3 } else { 2 } {
        if cycle == 2 {
            roots.truncate(2);
        }
        fixture.heap.collect(&roots, &[]).unwrap();
        let events = fixture.heap.take_finalizers();
        total_events += events.len();
        observations.push(fixture.observe(total_events));
        if cycle == 0 && rescue {
            assert_eq!(events.len(), 1);
            roots.push(Value::Userdata(events[0].userdata));
        }
    }
    observations
}

// ANCHOR: resurrection_real_vm
pub const VM_SETUP: &[u8] = br#"
prefix = { answer = 7 }
weak = setmetatable({}, { __mode = 'v' })
runs = 0
local function setup()
  local child = { deep = { answer = 42 } }
  local item = newuserdata()
  local function finalize(value)
    runs = runs + 1
    local probe = {}
    probe[value] = 42
    saw_weak = weak.userdata == value and weak.callback ~= nil
      and weak.child == child and weak.grandchild == child.deep
      and child.deep.answer == 42 and probe[value] == 42
    rescued = value
  end
  setfinalizer(item, finalize)
  weak.userdata = item
  weak.callback = finalize
  weak.child = child
  weak.grandchild = child.deep
end
setup()
setup = nil
gc_collect()
return saw_weak and runs == 1 and rescued ~= nil
"#;
// ANCHOR_END: resurrection_real_vm

pub fn vm_rescue_cycles() -> [Value; 3] {
    fn run(vm: &mut Vm, source: &[u8]) -> Value {
        vm.execute(&compile(source).unwrap()).unwrap()
    }
    let mut vm = Vm::new();
    [
        run(&mut vm, VM_SETUP),
        run(
            &mut vm,
            br#"
          gc_collect()
          local probe = {}
          probe[rescued] = 42
          return runs == 1 and weak.userdata == rescued and weak.callback ~= nil
            and weak.child.deep.answer == 42 and prefix.answer == 7
            and probe[rescued] == 42
        "#,
        ),
        run(
            &mut vm,
            br#"
          rescued = nil
          gc_collect()
          return runs == 1 and weak.userdata == nil and weak.callback == nil
            and weak.child == nil and weak.grandchild == nil and prefix.answer == 7
        "#,
        ),
    ]
}
