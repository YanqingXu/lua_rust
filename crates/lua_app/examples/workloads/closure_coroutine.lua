local phase, iterations, width = ...
if phase == 0 then
  bench_closures, bench_coroutines = {}, {}
  for i = 1, width do
    local box = {value = 0}
    bench_closures[i] = function(delta)
      box.value = box.value + delta
      return box.value
    end
    local co = coroutine.create(function()
      local total = 0 -- Remains an open local in a suspended state across batches/GC.
      while true do
        local delta = coroutine.yield(total)
        total = total + delta
      end
    end)
    bench_coroutines[i] = co
    local ok, value = coroutine.resume(co)
    assert(ok and value == 0)
  end
  return width
elseif phase == 2 then
  bench_closures, bench_coroutines = nil, nil
  return 0
end

local checksum = 0
for i = 1, iterations do
  local slot = ((i - 1) % width) + 1
  local expected = bench_closures[slot](1)
  local ok, value = coroutine.resume(bench_coroutines[slot], 1)
  assert(ok and value == expected)
  checksum = checksum + value
  -- Churn short-lived states and exercise a closure over an open upvalue,
  -- then over that same upvalue after its coroutine has returned.
  local co = coroutine.create(function()
    local box = {value = i}
    coroutine.yield(function() return box.value end)
    box.value = box.value + 1
  end)
  local started, get = coroutine.resume(co)
  assert(started and get() == i)
  assert(coroutine.resume(co))
  assert(coroutine.status(co) == 'dead' and get() == i + 1)
end
return checksum
