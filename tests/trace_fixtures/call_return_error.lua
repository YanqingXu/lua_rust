local function make_adder(delta)
    local function add(value)
        return value + delta
    end
    return add
end

local add_one = make_adder(1)

local worker = coroutine.create(function(value)
    local resumed = coroutine.yield(value + 1)
    return resumed + 1
end)
local resume_ok, yielded = coroutine.resume(worker, 10)
assert(resume_ok and yielded == 11)
local finish_ok, result = coroutine.resume(worker, 20)
assert(finish_ok and result == 21)

print("trace lifecycle", add_one(41), yielded, result)
error("trace lifecycle failure")
