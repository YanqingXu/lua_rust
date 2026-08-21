local values = { 9, 4, 1 }
table.sort(values)
assert(table.concat(values, ",") == "1,4,9")

local worker = coroutine.create(function(value)
    return math.sqrt(value)
end)
local ok, root = coroutine.resume(worker, 256)

return (ok and string.upper("lua") == "LUA")
    and root * 100 + string.len("Lua!") * 4 or 0
