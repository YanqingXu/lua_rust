local worker = coroutine.create(function(value)
    local resumed = coroutine.yield(value + 1)
    return resumed + 1
end)

local ok, yielded = coroutine.resume(worker, 10)
assert(ok and yielded == 11)

local resumed_ok, result = coroutine.resume(worker, 20)
assert(resumed_ok and result == 21)

print("coroutine lifecycle", yielded, result)
