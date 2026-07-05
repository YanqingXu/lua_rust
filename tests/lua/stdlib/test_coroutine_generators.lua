-- Test coroutine generator patterns
print("=== Testing coroutine generators ===")

-- Test 1: simple range generator
print("\nTest 1: range generator")
local function range(n)
    return coroutine.create(function()
        for i = 1, n do
            coroutine.yield(i)
        end
    end)
end

local co = range(5)
local r1, r2, r3, r4, r5
local _
_, r1 = coroutine.resume(co)
_, r2 = coroutine.resume(co)
_, r3 = coroutine.resume(co)
_, r4 = coroutine.resume(co)
_, r5 = coroutine.resume(co)
print("  range(5):", r1, r2, r3, r4, r5)

-- Test 2: fibonacci generator
print("\nTest 2: fibonacci generator")
local function fibonacci(n)
    return coroutine.create(function()
        local a, b = 0, 1
        for i = 1, n do
            coroutine.yield(a)
            local tmp = a
            a = b
            b = tmp + b
        end
    end)
end

local fib = fibonacci(8)
local f1, f2, f3, f4, f5, f6, f7, f8
local _
_, f1 = coroutine.resume(fib)
_, f2 = coroutine.resume(fib)
_, f3 = coroutine.resume(fib)
_, f4 = coroutine.resume(fib)
_, f5 = coroutine.resume(fib)
_, f6 = coroutine.resume(fib)
_, f7 = coroutine.resume(fib)
_, f8 = coroutine.resume(fib)
print("  fib(8):", f1, f2, f3, f4, f5, f6, f7, f8)
-- expected: 0 1 1 2 3 5 8 13

-- Test 3: producer-consumer pattern
print("\nTest 3: simple producer-consumer")
local function producer(n)
    return coroutine.create(function()
        for i = 1, n do
            coroutine.yield(i * i)
        end
    end)
end

local prod = producer(5)
local r1, r2, r3, r4, r5
local _, v
_, r1 = coroutine.resume(prod)
_, r2 = coroutine.resume(prod)
_, r3 = coroutine.resume(prod)
_, r4 = coroutine.resume(prod)
_, r5 = coroutine.resume(prod)
print("  squares:", r1, r2, r3, r4, r5)
-- expected: 1 4 9 16 25

-- Test 4: accumulator pattern
print("\nTest 4: accumulator")
local function accumulator()
    return coroutine.create(function()
        local sum = 0
        while true do
            local val = coroutine.yield(sum)
            if val == nil then return sum end
            sum = sum + val
        end
    end)
end

local acc = accumulator()
coroutine.resume(acc)  -- start, get 0
local _, s1 = coroutine.resume(acc, 10)
print("  add 10, sum:", s1)
local _, s2 = coroutine.resume(acc, 20)
print("  add 20, sum:", s2)
local _, s3 = coroutine.resume(acc, 30)
print("  add 30, sum:", s3)
-- expected: 10, 30, 60

print("\n=== Generator tests done ===")
