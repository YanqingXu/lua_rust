local runs = 0
local co = coroutine.create(function(seed)
    runs = runs + 1
    local resumed = coroutine.yield(seed + 1)
    return resumed + 2
end)

local ok1, first = coroutine.resume(co, 10)
local ok2, final = coroutine.resume(co, 40)
return ok1 and ok2 and first == 11 and final == 42 and runs == 1
