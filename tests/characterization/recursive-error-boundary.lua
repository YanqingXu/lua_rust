-- Run only in a subprocess: a VM regression must not kill a host test harness.
local function finite(n)
  if n == 0 then return 0 end
  return finite(n - 1) + 1
end
assert(finite(400) == 400, 'finite non-tail recursion lost a result')

local overflow_count, overflow_saved
local function overflow()
  overflow_count = overflow_count + 1
  local held = {value = overflow_count}
  if overflow_count == 64 then
    overflow_saved = function() return held.value end
  end
  overflow()
  return held.value
end
local function check_overflow()
  overflow_count, overflow_saved = 0, nil
  local ok, err = pcall(overflow)
  assert(not ok and type(err) == 'string' and string.find(err, 'stack overflow', 1, true), tostring(err))
  assert(overflow_count >= 64, 'recursion failed before the exercised live-frame boundary')
  collectgarbage('collect')
  assert(overflow_saved() == 64, 'error unwind failed to close a captured local')
  assert(finite(20) == 20, 'protected overflow did not restore execution')
end
check_overflow()
check_overflow()
check_overflow()

local function tail(n, total)
  if n == 0 then return total end
  return tail(n - 1, total + 1)
end
assert(tail(10000, 0) == 10000, 'tail calls consumed ordinary recursive frames')

local callable = setmetatable({}, {__call = function(self, n)
  if n == 0 then return 0 end
  return self(n and n - 1) + 1
end})
assert(callable(120) == 120, 'non-tail __call recursion lost a result')
local ok, err = pcall(function() return callable() end)
assert(not ok and type(err) == 'string' and string.find(err, 'stack overflow', 1, true), tostring(err))
assert(callable(20) == 20, '__call overflow did not restore execution')

local token = {}
local saved
local function capture_then_fail(n)
  local held = {value = n}
  if n == 17 then saved = function() return held.value end end
  if n == 0 then error(token, 0) end
  return capture_then_fail(n - 1) + held.value
end
local ok, err = pcall(capture_then_fail, 120)
assert(not ok and rawequal(err, token), 'deep protected error object changed')
collectgarbage('collect')
assert(saved() == 17, 'explicit error unwind lost an upvalue')

local calls, returns = 0, 0
debug.sethook(function(event)
  if event == 'call' then calls = calls + 1 end
  if event == 'return' then returns = returns + 1 end
end, 'cr')
local hooked = finite(80)
debug.sethook()
assert(hooked == 80 and calls >= 80 and returns >= 80, 'recursive hook delivery or result changed')

local function rooted(n)
  local held = {value = n}
  if n == 0 then collectgarbage('collect'); return 0 end
  local result = rooted(n - 1)
  assert(held.value == n, 'collection lost a live recursive frame')
  return result + 1
end
assert(rooted(160) == 160, 'recursive collection changed return values')

-- Lua TFOR callbacks suspend internally for protected calls, GC and hooks.
-- Each return must update the hidden control register once; nil must exit.
local iterator_calls, iterator_hooks, iterator_returns = 0, 0, 0
local function iterator_result(state, control, value)
  if control >= state.limit then return nil end
  return control + 1, value + control
end
local function iterator(state, control)
  assert(control == iterator_calls, 'TFOR control value was not restored')
  iterator_calls = iterator_calls + 1
  if control >= state.limit then return iterator_result(state, control) end
  local held = {control = control}
  local ok, value = pcall(finite, 40)
  assert(ok and value == 40)
  collectgarbage('collect')
  assert(held.control == control, 'iterator suspension lost a live value')
  return iterator_result(state, control, value)
end
debug.sethook(function(event)
  if event == 'call' then iterator_hooks = iterator_hooks + 1 end
  if event == 'return' then iterator_returns = iterator_returns + 1 end
end, 'cr')
local seen, sum = 0, 0
for index, value in iterator, {limit = 3}, 0 do
  assert(index == seen + 1, 'TFOR result registers were not restored')
  seen, sum = index, sum + value
end
debug.sethook()
assert(seen == 3 and sum == 123 and iterator_calls == 4, 'TFOR did not terminate at nil')
assert(iterator_hooks >= 3 and iterator_returns >= 3, 'iterator call/return hooks were not delivered')

local function suspended(n)
  local held = {value = n}
  if n == 0 then return coroutine.yield('paused') end
  local result = suspended(n - 1)
  assert(held.value == n, 'suspended recursive frame lost a live value')
  return result + 1
end
local co = coroutine.create(function()
  local value = suspended(80)
  check_overflow()
  return value, 'finished'
end)
local ok, value = coroutine.resume(co)
assert(ok and value == 'paused', tostring(value))
collectgarbage('collect')
local ok, value, tag = coroutine.resume(co, 10)
assert(ok and value == 90 and tag == 'finished', tostring(value))
assert(coroutine.status(co) == 'dead' and finite(20) == 20)

print('recursive error boundary continued')
return 'recursive boundary returned'
