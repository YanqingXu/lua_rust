local worker = coroutine.create(function(seed)
    local resumed = coroutine.yield(seed + 1, seed + 2)
    return resumed + 3
end)
local ok1, first, second = coroutine.resume(worker, 10)
local ok2, final = coroutine.resume(worker, 39)
print(ok1, first, second, ok2, final, coroutine.status(worker))
