-- Test coroutine with closures and upvalues
print("=== Testing coroutine closures ===")

-- Test 1: coroutine body as closure with upvalue
print("\nTest 1: closure with upvalue")
local counter = 0
local co = coroutine.create(function()
    for i = 1, 3 do
        counter = counter + 1
        coroutine.yield(counter)
    end
end)

local _, v1 = coroutine.resume(co)
local _, v2 = coroutine.resume(co)
local _, v3 = coroutine.resume(co)
print("  counter values:", v1, v2, v3)  -- 1, 2, 3
print("  final counter:", counter)       -- 3

-- Test 2: coroutine factory with closure
print("\nTest 2: coroutine factory")
local function make_counter(start, step)
    local val = start
    return coroutine.create(function()
        while true do
            coroutine.yield(val)
            val = val + step
        end
    end)
end

local c1 = make_counter(0, 2)
local c2 = make_counter(100, 10)

local _, a1 = coroutine.resume(c1)
local _, b1 = coroutine.resume(c2)
local _, a2 = coroutine.resume(c1)
local _, b2 = coroutine.resume(c2)
local _, a3 = coroutine.resume(c1)
print("  c1:", a1, a2, a3)      -- 0, 2, 4
print("  c2:", b1, b2)           -- 100, 110

-- Test 3: shared state between coroutines via upvalue
print("\nTest 3: shared state")
local shared = {value = 0}

local writer = coroutine.create(function()
    for i = 1, 3 do
        shared.value = shared.value + 10
        coroutine.yield()
    end
end)

local reader = coroutine.create(function()
    local readings = {}
    for i = 1, 3 do
        readings[i] = shared.value
        coroutine.yield(readings[i])
    end
end)

coroutine.resume(writer)  -- shared.value = 10
local _, r1 = coroutine.resume(reader)
coroutine.resume(writer)  -- shared.value = 20
local _, r2 = coroutine.resume(reader)
coroutine.resume(writer)  -- shared.value = 30
local _, r3 = coroutine.resume(reader)
print("  readings:", r1, r2, r3)  -- 10, 20, 30

-- Test 4: nested function calls inside coroutine
print("\nTest 4: nested function calls")
local function helper(x)
    return x * x
end

local co4 = coroutine.create(function(n)
    local sum = 0
    for i = 1, n do
        sum = sum + helper(i)
        coroutine.yield(sum)
    end
    return sum
end)

local _, s1 = coroutine.resume(co4, 4)
local _, s2 = coroutine.resume(co4)
local _, s3 = coroutine.resume(co4)
local _, s4 = coroutine.resume(co4)
print("  partial sums:", s1, s2, s3, s4)  -- 1, 5, 14, 30

print("\n=== Closure tests done ===")
