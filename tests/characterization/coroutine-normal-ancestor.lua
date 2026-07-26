local sequence = 0
local a
local b

local function event(label, value1, value2, value3, value4)
    sequence = sequence + 1
    print(sequence, label, value1, value2, value3, value4)
end

a = coroutine.create(function(input)
    event("A:enter", input, coroutine.status(a), coroutine.status(b), "-")

    local ok, value = coroutine.resume(b, "from-A")
    event("A:after-B", ok, value, coroutine.status(a), coroutine.status(b))

    return "A-done"
end)

b = coroutine.create(function(input)
    event("B:enter", input, coroutine.status(a), coroutine.status(b), "-")

    local ok, value = coroutine.resume(a, "from-B")
    if not ok then
        value = "<resume-error>"
    end
    event("B:after-A", ok, value, coroutine.status(a), coroutine.status(b))

    return "B-done"
end)

event("main:before", coroutine.status(a), coroutine.status(b), "-", "-")
local ok, value = coroutine.resume(a, "from-main")
if not ok then
    value = "<resume-error>"
end
event("main:after-A", ok, value, coroutine.status(a), coroutine.status(b))
