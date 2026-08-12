local returns_immediately = coroutine.create(function()
end)

local returned_ok, returned_value = coroutine.resume(returns_immediately)
assert(returned_ok and returned_value == nil)

local calls_immediately
do
    local function echo(value)
        return value
    end

    calls_immediately = coroutine.create(function()
        local value = echo(coroutine.yield("ready"))
        return value
    end)
end

local started_ok, yielded = coroutine.resume(calls_immediately)
assert(started_ok and yielded == "ready")

local resumed_ok, result = coroutine.resume(calls_immediately, 42)
assert(resumed_ok and result == 42)

print("coroutine control-first resume", returned_ok, yielded, result)
error("coroutine control-first resume complete")
