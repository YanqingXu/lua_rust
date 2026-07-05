-- Test coroutine basic functionality
print("=== Testing coroutine basic ===")

-- Test 1: coroutine.create
print("\nTest 1: coroutine.create")
local co = coroutine.create(function()
    print("  inside coroutine")
end)
print("  type:", type(co))
print("  status:", coroutine.status(co))

-- Test 2: basic resume
print("\nTest 2: basic resume")
local ok = coroutine.resume(co)
print("  resume result:", ok)
print("  status after:", coroutine.status(co))

-- Test 3: basic yield
print("\nTest 3: basic yield")
local co2 = coroutine.create(function()
    print("  before yield")
    coroutine.yield()
    print("  after yield")
end)
print("  first resume:")
coroutine.resume(co2)
print("  status:", coroutine.status(co2))
print("  second resume:")
coroutine.resume(co2)
print("  status:", coroutine.status(co2))

-- Test 4: yield with values
print("\nTest 4: yield with values")
local co3 = coroutine.create(function()
    coroutine.yield(10)
    coroutine.yield(20)
    return 30
end)
local ok1, v1 = coroutine.resume(co3)
print("  resume 1:", ok1, v1)
local ok2, v2 = coroutine.resume(co3)
print("  resume 2:", ok2, v2)
local ok3, v3 = coroutine.resume(co3)
print("  resume 3:", ok3, v3)

-- Test 5: resume with arguments (first resume = function args)
print("\nTest 5: resume with initial arguments")
local co4 = coroutine.create(function(a, b)
    print("  args:", a, b)
    return a + b
end)
local ok4, v4 = coroutine.resume(co4, 3, 7)
print("  result:", ok4, v4)

-- Test 6: resume dead coroutine
print("\nTest 6: resume dead coroutine")
local ok5, err5 = coroutine.resume(co4)
print("  result:", ok5, err5)

print("\n=== All basic tests done ===")
