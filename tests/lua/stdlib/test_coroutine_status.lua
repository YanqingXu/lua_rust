-- Test coroutine status and running
print("=== Testing coroutine status/running ===")

-- Test 1: status lifecycle
print("\nTest 1: status lifecycle")
local co = coroutine.create(function()
    coroutine.yield()
end)
print("  before resume:", coroutine.status(co))   -- suspended
coroutine.resume(co)
print("  after yield:", coroutine.status(co))      -- suspended
coroutine.resume(co)
print("  after return:", coroutine.status(co))     -- dead

-- Test 2: status inside coroutine (running)
print("\nTest 2: status inside coroutine")
local inner_status = nil
local co2 = coroutine.create(function()
    inner_status = coroutine.status(coroutine.running())
    coroutine.yield()
end)
coroutine.resume(co2)
print("  status from inside:", inner_status)  -- running

-- Test 3: coroutine.running() in main thread
print("\nTest 3: running in main thread")
local main_running = coroutine.running()
print("  main thread running:", main_running)  -- nil

-- Test 4: coroutine.running() inside coroutine
print("\nTest 4: running inside coroutine")
local captured_thread = nil
local co3 = coroutine.create(function()
    captured_thread = coroutine.running()
end)
coroutine.resume(co3)
print("  captured thread is not nil:", captured_thread ~= nil)

-- Test 5: status after error
print("\nTest 5: status after error")
local co4 = coroutine.create(function()
    error("test error")
end)
local ok, err = coroutine.resume(co4)
print("  resume ok:", ok)
print("  status:", coroutine.status(co4))  -- dead

-- Test 6: multiple coroutines independent status
print("\nTest 6: multiple independent coroutines")
local co_a = coroutine.create(function()
    coroutine.yield(1)
    coroutine.yield(2)
end)
local co_b = coroutine.create(function()
    coroutine.yield(10)
    coroutine.yield(20)
end)

local _, va1 = coroutine.resume(co_a)
local _, vb1 = coroutine.resume(co_b)
print("  co_a yield 1:", va1, "status:", coroutine.status(co_a))
print("  co_b yield 1:", vb1, "status:", coroutine.status(co_b))

local _, va2 = coroutine.resume(co_a)
print("  co_a yield 2:", va2, "status:", coroutine.status(co_a))

coroutine.resume(co_a)
print("  co_a final status:", coroutine.status(co_a))  -- dead
print("  co_b still:", coroutine.status(co_b))          -- suspended

print("\n=== Status/running tests done ===")
