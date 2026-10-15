//! 真实 VM 探针：一次 execute 内消费错误并继续，不读取或改写 VM 私有 roots。
use lua_compiler::compile;
use lua_core::Value;
use lua_vm::Vm;

pub fn run(source: &[u8]) -> Vec<Value> {
    Vm::new().execute_multi(&compile(source).unwrap()).unwrap()
}

pub fn pcall_multi_values() -> Vec<Value> {
    run(br#"
        local function relay(...) return select('#', ...), ... end
        return relay(pcall(function() return 7, nil, 9, nil end))
    "#)
}

// ANCHOR: protected_real_same_execute
pub const ERROR_THEN_CONTINUE: &[u8] = br#"
local weak = setmetatable({}, { __mode = 'v' })
local prefix = { answer = 41 }
local caught = false
do
  local ok, message = pcall(function(value)
    weak.argument = value
    gc_collect()
    error('body failed')
  end, { answer = 42 })
  caught = not ok and type(message) == 'string'
    and string.find(message, 'body failed', 1, true) ~= nil
end
gc_collect()
local later_ok, later_value = pcall(function(value)
  gc_collect()
  return value.answer + 1
end, prefix)
return caught, later_ok, later_value, prefix.answer, weak.argument == nil
"#;
// ANCHOR_END: protected_real_same_execute

pub fn xpcall_success_skips_handler() -> Vec<Value> {
    run(br#"
        local calls = 0
        local ok, a, hole, b, tail = xpcall(function() return 7,nil,9,nil end,
          function() calls = calls + 1 return 99 end)
        return ok,a,hole,b,tail,calls
    "#)
}

pub fn xpcall_handler_variants() -> Vec<Value> {
    run(br#"
        local calls = 0
        local seen_string = false
        local a, first, discarded = xpcall(function() error('body') end,
          function(message)
            calls = calls + 1
            seen_string = type(message) == 'string'
            return 42,99
          end)
        local b, empty = xpcall(function() error('body') end,function() end)
        local c, nil_first, ignored = xpcall(function() error('body') end,
          function() return nil,99 end)
        local d, handler_error = xpcall(function() error('body') end,
          function() error('handler failed') end)
        local e, object = xpcall(function() error('body') end,
          function() return { answer = 42 },99 end)
        gc_collect()
        local continued = 1 + 1
        return not a,first,discarded==nil,calls,seen_string,not b,empty==nil,
          not c,nil_first==nil,ignored==nil,not d,type(handler_error)=='string',
          string.find(handler_error,'handler failed',1,true)~=nil,continued,
          not e,object.answer
    "#)
}

pub fn errors_are_formatted_strings() -> Vec<Value> {
    run(br#"
        local payload = { answer = 42 }
        local a, message = pcall(function() error(payload) end)
        local seen_string = false
        local b, first = xpcall(function() error(payload) end,function(value)
          seen_string = type(value) == 'string'
          return 42
        end)
        local bad, bad_message = pcall(17)
        return not a,type(message)=='string',message~=payload,
          string.find(message,'table',1,true)~=nil,
          not b,seen_string,first,not bad,type(bad_message)=='string',payload.answer
    "#)
}
