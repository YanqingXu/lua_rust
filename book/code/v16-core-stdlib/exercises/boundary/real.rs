//! Same-execute probes of unchanged VM boundaries; no private depth/roots inspection.
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;

// ANCHOR: boundary_real_afterwards
fn followed_by_legal_yield(checks: &[u8]) -> Vec<Value> {
    let mut source = br#"
      local co = coroutine.create(function()
        local checked = (function()
    "#
    .to_vec();
    source.extend_from_slice(checks);
    source.extend_from_slice(
        br#"
        end)()
        local resumed = coroutine.yield(checked, 41, nil)
        return checked and resumed == 42, 99
      end)
      local ok1, checked, tag, hole = coroutine.resume(co)
      local ok2, final, answer = coroutine.resume(co, 42)
      return ok1, checked, tag, hole, ok2, final, answer,
        coroutine.status(co) == 'dead'
    "#,
    );
    let proto = compile(&source).unwrap();
    Vm::new().execute_multi(&proto).unwrap()
}
// ANCHOR_END: boundary_real_afterwards

pub fn pcall_then_yield() -> Vec<Value> {
    followed_by_legal_yield(
        br#"
      local failed, ordinary = pcall(function() error('ordinary failure') end)
      local denied, message = pcall(function() return coroutine.yield('forbidden pcall') end)
      return not failed and type(ordinary) == 'string' and not denied
        and string.find(message, 'boundary') ~= nil
    "#,
    )
}

pub fn xpcall_then_yield() -> Vec<Value> {
    followed_by_legal_yield(br#"
      local failed, handled = xpcall(function() error('ordinary failure') end, function() return 17 end)
      local denied, message = xpcall(function() return coroutine.yield('forbidden xpcall') end, function(e) return e end)
      local hfailed, hmessage = xpcall(function() error('handler trigger') end, function() return coroutine.yield('forbidden handler') end)
      return not failed and handled == 17 and not denied
        and string.find(message, 'boundary') ~= nil and not hfailed
        and string.find(hmessage, 'boundary') ~= nil
    "#)
}

pub fn sort_then_yield() -> Vec<Value> {
    followed_by_legal_yield(
        br#"
      local failed, ordinary = pcall(function()
        table.sort({2, 1}, function() error('sort failure') end)
      end)
      local denied, message = pcall(function()
        table.sort({2, 1}, function() return coroutine.yield('forbidden comparator') end)
      end)
      return not failed and string.find(ordinary, 'sort failure') ~= nil
        and not denied and string.find(message, 'boundary') ~= nil
    "#,
    )
}

pub fn callable_metamethod_then_yield() -> Vec<Value> {
    followed_by_legal_yield(br#"
      local good = setmetatable({}, {__call = function() return 42 end})
      local bad = setmetatable({}, {__call = function() return coroutine.yield('forbidden __call') end})
      local ok, answer = pcall(good)
      local denied, message = pcall(bad)
      return ok and answer == 42 and not denied and string.find(message, 'boundary') ~= nil
    "#)
}

pub fn nested_pcall_then_yield() -> Vec<Value> {
    followed_by_legal_yield(
        br#"
      local denied, message = pcall(function()
        local inner = pcall(function() error('inner failure') end)
        if inner then error('inner should fail') end
        return coroutine.yield('outer still forbids yield')
      end)
      return not denied and string.find(message, 'boundary') ~= nil
    "#,
    )
}
