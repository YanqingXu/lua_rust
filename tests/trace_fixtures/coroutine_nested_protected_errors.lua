local leaf = coroutine.create(function(value)
    local captured = value
    local function advance(next_value)
        captured = next_value
        return captured + 1
    end

    local resumed = coroutine.yield(advance(value))
    return advance(resumed)
end)

local wrapped = coroutine.wrap(function(thread, value)
    local leaf_started, yielded = coroutine.resume(thread, value)
    assert(leaf_started and yielded == 11)

    local protected, message = pcall(function()
        error("caught inside wrapped coroutine")
    end)
    assert(not protected and string.find(message, "caught inside wrapped coroutine", 1, true))

    local running = coroutine.running()
    local invalid_ok, invalid_message = coroutine.resume(running)
    assert(not invalid_ok and type(invalid_message) == "string")

    local resumed = coroutine.yield(yielded + 1)
    local leaf_done, result = coroutine.resume(thread, resumed)
    assert(leaf_done and result == resumed + 1)
    return result + 1
end)

local first = wrapped(leaf, 10)
assert(first == 12)
local final = wrapped(20)
assert(final == 22)

local dead_ok, dead_message = coroutine.resume(leaf)
assert(not dead_ok and type(dead_message) == "string")

print("nested protected coroutine errors", first, final, dead_ok)
error("nested protected coroutine errors complete")
