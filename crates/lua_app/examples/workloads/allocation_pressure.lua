local phase, iterations, width, batch = ...
if phase == 0 then
  bench_live = {}
  return 0
elseif phase == 2 then
  bench_live = nil
  return 0
end

local first = (batch - 1) * iterations
for i = 1, iterations do
  local serial = first + i
  local slot = ((serial - 1) % width) + 1
  -- A bounded survivor window amid tables, nested arrays, and unique strings.
  bench_live[slot] = {
    serial = serial,
    payload = 'object:' .. serial .. ':' .. string.rep('x', 64),
    children = {{serial + 1, serial + 2}, {serial + 3, serial + 4}}
  }
end
local count, checksum = 0, 0
for _, object in pairs(bench_live) do
  assert(#object.payload >= 73)
  assert(object.children[1][1] == object.serial + 1)
  assert(object.children[2][2] == object.serial + 4)
  count, checksum = count + 1, checksum + object.serial
end
assert(count == math.min(width, batch * iterations))
return checksum
