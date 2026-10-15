//! Probes of the unchanged v0.15 VM; these execute Lua rather than the toy scheduler.
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;

pub fn interleaved_locals() -> Vec<Value> {
    let proto = compile(
        br#"
      local function body(seed)
        local value = seed
        local delta = coroutine.yield(value, nil)
        value = value + delta
        local last = coroutine.yield(value, nil)
        return value + last, nil
      end
      local first, second = coroutine.create(body), coroutine.create(body)
      local o1, a, n1 = coroutine.resume(first, 10)
      local o2, b, n2 = coroutine.resume(second, 100)
      local o3, c, n3 = coroutine.resume(first, 1)
      local o4, d, n4 = coroutine.resume(second, 2)
      local o5, e, n5 = coroutine.resume(first, 3)
      local o6, f, n6 = coroutine.resume(second, 4)
      return o1 and o2 and o3 and o4 and o5 and o6,
        a, b, c, d, e, f,
        n1 == nil and n2 == nil and n3 == nil and n4 == nil
          and n5 == nil and n6 == nil and coroutine.status(first) == 'dead'
          and coroutine.status(second) == 'dead'
    "#,
    )
    .unwrap();
    Vm::new().execute_multi(&proto).unwrap()
}

// ANCHOR: co_real_nested_yields
pub fn nested_calls_and_multiple_yields() -> [Vec<Value>; 3] {
    let mut vm = Vm::new();
    let setup = compile(
        br#"
      effects = 0
      local function inner()
        local resumed = coroutine.yield(10, nil)
        return resumed + 1
      end
      saved = coroutine.create(function()
        effects = effects + 1
        local first = inner()
        local second = coroutine.yield(first, nil)
        return second + 2, effects, nil
      end)
      return coroutine.resume(saved)
    "#,
    )
    .unwrap();
    let second = compile(b"return coroutine.resume(saved, 20)").unwrap();
    let third = compile(b"return coroutine.resume(saved, 30)").unwrap();
    [
        vm.execute_multi(&setup).unwrap(),
        vm.execute_multi(&second).unwrap(),
        vm.execute_multi(&third).unwrap(),
    ]
}
// ANCHOR_END: co_real_nested_yields

pub fn nested_statuses_and_value_direction() -> (Vec<Value>, Value) {
    let mut vm = Vm::new();
    let proto = compile(
        br#"
      inner = coroutine.create(function(parent)
        local parent_normal = coroutine.status(parent) == 'normal'
        local me = coroutine.running() == inner and coroutine.status(inner) == 'running'
        local x, y, z = coroutine.yield(parent_normal, me, nil)
        return x, y, z, coroutine.status(parent) == 'normal'
      end)
      outer = coroutine.create(function()
        local ok1, normal, me, hole = coroutine.resume(inner, outer)
        local restored = coroutine.running() == outer and coroutine.status(outer) == 'running'
        local ok2, x, y, z, normal2 = coroutine.resume(inner, 7, nil, 9)
        restored = restored and coroutine.running() == outer and coroutine.status(outer) == 'running'
        return ok1, normal, me, hole, restored, ok2, x, y, z, normal2
      end)
      return coroutine.resume(outer)
    "#,
    )
    .unwrap();
    let values = vm.execute_multi(&proto).unwrap();
    let final_state = compile(b"return coroutine.status(inner) == 'dead' and coroutine.status(outer) == 'dead' and coroutine.running() == nil").unwrap();
    (values, vm.execute(&final_state).unwrap())
}
