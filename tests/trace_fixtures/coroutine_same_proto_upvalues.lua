local function worker(seed)
    local captured = seed
    local function bump(delta)
        captured = captured + delta
        return captured
    end

    local first = bump(1)
    local resumed = coroutine.yield(first)
    return bump(resumed)
end

local left = coroutine.create(worker)
local right = coroutine.create(worker)

local left_started, left_first = coroutine.resume(left, 10)
assert(left_started and left_first == 11)
local right_started, right_first = coroutine.resume(right, 20)
assert(right_started and right_first == 21)
assert(coroutine.status(left) == "suspended")
assert(coroutine.status(right) == "suspended")

local left_done, left_final = coroutine.resume(left, 3)
assert(left_done and left_final == 14)
assert(coroutine.status(left) == "dead")
assert(coroutine.status(right) == "suspended")

local right_done, right_final = coroutine.resume(right, 4)
assert(right_done and right_final == 25)
assert(coroutine.status(right) == "dead")

print("same-proto coroutine upvalues", left_first, right_first, left_final, right_final)
error("same-proto coroutine upvalues complete")
