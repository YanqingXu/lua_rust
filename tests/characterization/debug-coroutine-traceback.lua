-- Check retained frame structure without fixing engine-specific source formatting.
local function check_frames(co, expected)
  local lines = {}
  local traceback = debug.traceback(co)
  for line in string.gmatch(traceback, '[^\r\n]+') do
    lines[#lines + 1] = line
  end
  assert(lines[1] == 'stack traceback:', traceback)
  assert(#lines == #expected + 1, traceback)
  for i, kind in ipairs(expected) do
    local line = lines[i + 1]
    if kind == 'lua' then
      assert(not string.find(line, '[C]', 1, true), traceback)
      assert(string.find(line, ': in function ', 1, true), traceback)
    elseif kind == 'tail' then
      assert(string.find(line, '(tail call)', 1, true), traceback)
    else
      assert(string.find(line, '[C]', 1, true), traceback)
      assert(string.find(line, "'" .. kind .. "'", 1, true), traceback)
    end
  end
end

-- A plain failure has one Lua frame, even after collection and repeated reads.
local co = coroutine.create(function() error('ordinary failure') end)
local ok = coroutine.resume(co)
assert(not ok and coroutine.status(co) == 'dead')
check_frames(co, {'error', 'lua'})
collectgarbage('collect')
check_frames(co, {'error', 'lua'})

-- Four recursive invocations plus the entry closure must appear exactly once.
local function fail(n)
  if n == 0 then error('recursive failure') end
  fail(n - 1)
end
co = coroutine.create(function() fail(3) end)
ok = coroutine.resume(co)
assert(not ok and coroutine.status(co) == 'dead')
check_frames(co, {'error', 'lua', 'lua', 'lua', 'lua', 'lua'})

-- The native frame changes from yield to error while the real Lua frames grow.
local function yield_then_fail(n)
  if n == 0 then error('failure after yields') end
  coroutine.yield(n)
  yield_then_fail(n - 1)
end
co = coroutine.create(function() yield_then_fail(3) end)
local expected = {'yield', 'lua', 'lua'}
for n = 3, 1, -1 do
  local ok, value = coroutine.resume(co)
  assert(ok and value == n and coroutine.status(co) == 'suspended')
  collectgarbage('collect')
  check_frames(co, expected)
  table.insert(expected, 2, 'lua')
end
ok = coroutine.resume(co)
assert(not ok and coroutine.status(co) == 'dead')
expected[1] = 'error'
check_frames(co, expected)
collectgarbage('collect')
check_frames(co, expected)

-- Tail calls retain logical tail markers, then disappear on a normal return.
local function tail(n)
  if n > 0 then return tail(n - 1) end
  coroutine.yield('tail paused')
end
co = coroutine.create(tail)
local ok, value = coroutine.resume(co, 3)
assert(ok and value == 'tail paused' and coroutine.status(co) == 'suspended')
check_frames(co, {'yield', 'lua', 'tail', 'tail', 'tail'})
ok = coroutine.resume(co)
assert(ok and coroutine.status(co) == 'dead')
check_frames(co, {})
collectgarbage('collect')
check_frames(co, {})

print('debug coroutine traceback passed')
