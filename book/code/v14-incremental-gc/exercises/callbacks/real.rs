//! 对照真实 v0.14：不修改 Heap/VM，也不把模型输出冒充真实内部状态。
use lua_compiler::compile;
use lua_core::{Heap, Value};
use lua_vm::{Vm, VmErrorKind};
use std::rc::Rc;

// ANCHOR: callback_real_pair
pub fn vm_reentrant_pair() -> Value {
    let proto = compile(
        b"local sequence = 0
          do
            local first = newuserdata()
            setfinalizer(first, function()
              sequence = sequence * 10 + 1
              gc_collect()
              gc_collect()
              sequence = sequence * 10 + 2
            end)
            local second = newuserdata()
            setfinalizer(second, function(value)
              setfinalizer(value, nil)
              sequence = sequence * 10 + 3
            end)
          end
          gc_collect()
          gc_collect()
          return sequence",
    )
    .unwrap();
    Vm::new().execute(&proto).unwrap()
}
// ANCHOR_END: callback_real_pair

pub fn vm_error_then_sibling() -> (VmErrorKind, Value) {
    let proto = compile(
        b"sibling_after_error = false
          do
            local first = newuserdata()
            setfinalizer(first, function() return {} + 1 end)
            local second = newuserdata()
            setfinalizer(second, function(value)
              setfinalizer(value, nil)
              sibling_after_error = true
            end)
          end
          gc_collect()",
    )
    .unwrap();
    let mut vm = Vm::new();
    let error = vm.execute(&proto).unwrap_err();
    let probe = compile(b"return sibling_after_error").unwrap();
    (error.kind, vm.execute(&probe).unwrap())
}

/// 保留 Rust owned events，再跑完整下一轮 GC；返回兄弟 userdata/closure 是否可访问。
pub fn heap_owned_batch_accessible(pin_whole_batch: bool) -> (bool, bool) {
    let mut heap = Heap::new();
    let proto = Rc::new(compile(b"return nil").unwrap());
    for _ in 0..2 {
        let userdata = heap.allocate_userdata().unwrap();
        let callback = heap.allocate_function(Rc::clone(&proto)).unwrap();
        heap.userdata_set_finalizer(userdata, Some(Value::Function(callback)))
            .unwrap();
    }
    heap.collect(&[], &[]).unwrap();
    let events = heap.take_finalizers();
    assert_eq!(events.len(), 2);
    assert!(heap.take_finalizers().is_empty());
    let count = if pin_whole_batch { events.len() } else { 1 };
    let roots = events[..count]
        .iter()
        .flat_map(|event| [event.callback.clone(), Value::Userdata(event.userdata)])
        .collect::<Vec<_>>();
    heap.collect(&roots, &[]).unwrap();
    let Value::Function(callback) = events[1].callback else {
        panic!("fixture uses Lua closures");
    };
    let accessible = (
        heap.userdata_is_finalized(events[1].userdata).is_ok(),
        heap.function_proto(callback).is_ok(),
    );
    // 这个读取发生在 GC 之后：events 仍由 Rust 拥有，却不能阻止其中的 handle 失效。
    assert_eq!(events.len(), 2);
    accessible
}
