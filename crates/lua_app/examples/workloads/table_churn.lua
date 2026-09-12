local phase, iterations, width, batch, rotating = ...
if phase == 0 then
  bench_table, bench_keys = {}, {}
  for i = 1, width do
    bench_keys[i] = -i -- Force the hash part, without allocating strings.
    bench_table[-i] = i
  end
  return width
elseif phase == 2 then
  bench_table, bench_keys = nil, nil
  return 0
end

local first = width + (batch - 1) * iterations
for i = 1, iterations do
  local slot = ((i - 1) % width) + 1
  local key = bench_keys[slot]
  assert(bench_table[key] ~= nil)
  bench_table[key] = nil
  if rotating == 1 then
    key = -(first + i)
    bench_keys[slot] = key
  end
  bench_table[key] = first + i
end
local count, checksum = 0, 0
for key, value in pairs(bench_table) do
  assert(key < 0 and value > 0)
  count, checksum = count + 1, checksum + value
end
assert(count == width)
for i = 1, width do assert(bench_table[bench_keys[i]] ~= nil) end
return checksum
