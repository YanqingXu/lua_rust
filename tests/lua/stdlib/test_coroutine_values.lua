-- Test coroutine value passing (resume args become yield returns)
print("=== Testing coroutine value passing ===")

-- Test 1: resume args become yield returns
print("\nTest 1: resume args -> yield returns")
local co = coroutine.create(function(a)
    print("  initial arg:", a)
    local b = coroutine.yield(a * 2)
    print("  yield returned:", b)
    local c = coroutine.yield(b * 2)
    print("  yield returned:", c)
    return c * 2
end)

local ok1, v1 = coroutine.resume(co, 5)
print("  resume(co, 5) ->", ok1, v1)     -- true, 10

local ok2, v2 = coroutine.resume(co, 10)
print("  resume(co, 10) ->", ok2, v2)    -- true, 20

local ok3, v3 = coroutine.resume(co, 100)
print("  resume(co, 100) ->", ok3, v3)   -- true, 200

-- Test 2: multiple yield values
print("\nTest 2: multiple yield values")
local co2 = coroutine.create(function()
    coroutine.yield(1, 2, 3)
    coroutine.yield("a", "b")
    return "x", "y", "z"
end)

local ok, a, b, c = coroutine.resume(co2)
print("  yield 1:", ok, a, b, c)

ok, a, b = coroutine.resume(co2)
print("  yield 2:", ok, a, b)

ok, a, b, c = coroutine.resume(co2)
print("  return:", ok, a, b, c)

-- Test 3: no args on resume (nil yield return)
print("\nTest 3: no args resume")
local co3 = coroutine.create(function()
    local x = coroutine.yield()
    print("  yield returned:", x)
    return x == nil
end)
coroutine.resume(co3)
local ok4, v4 = coroutine.resume(co3)
print("  is nil:", ok4, v4)

print("\n=== Value passing tests done ===")
