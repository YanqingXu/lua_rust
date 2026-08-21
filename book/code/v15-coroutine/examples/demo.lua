local generator = coroutine.create(function(seed)
    local first = coroutine.yield(seed + 1)
    local second = coroutine.yield(first + 2)
    return second + 3
end)

local ok1, a = coroutine.resume(generator, 14)
local ok2, b = coroutine.resume(generator, 149)
gc_collect()
local ok3, c = coroutine.resume(generator, 1512)

return (ok1 and ok2 and ok3 and a == 15 and b == 151
    and coroutine.status(generator) == "dead") and c or 0
