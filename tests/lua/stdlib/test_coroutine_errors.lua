-- Test coroutine error handling
print("=== Testing coroutine error handling ===")

-- Test 1: error() inside coroutine
print("\nTest 1: error inside coroutine")
local co1 = coroutine.create(function()
    error("boom")
end)
local ok, err = coroutine.resume(co1)
print("  ok:", ok)
print("  error:", err)
print("  status:", coroutine.status(co1))

-- Test 2: error after yield
print("\nTest 2: error after yield")
local co2 = coroutine.create(function()
    coroutine.yield(1)
    error("delayed boom")
end)
local ok1, v1 = coroutine.resume(co2)
print("  first resume:", ok1, v1)
local ok2, err2 = coroutine.resume(co2)
print("  second resume:", ok2, err2)
print("  status:", coroutine.status(co2))

-- Test 3: resume dead coroutine (after error)
print("\nTest 3: resume dead after error")
local ok3, err3 = coroutine.resume(co2)
print("  resume dead:", ok3, err3)

-- Test 4: resume dead coroutine (after normal return)
print("\nTest 4: resume dead after normal return")
local co3 = coroutine.create(function() return 42 end)
coroutine.resume(co3)
local ok4, err4 = coroutine.resume(co3)
print("  resume dead:", ok4, err4)

-- Test 5: pcall inside coroutine
print("\nTest 5: pcall inside coroutine")
local co4 = coroutine.create(function()
    local ok, err = pcall(function()
        error("inner error")
    end)
    coroutine.yield(ok, err)
    return "survived"
end)
local rok, pcall_ok, pcall_err = coroutine.resume(co4)
print("  pcall caught:", rok, pcall_ok)
local rok2, final = coroutine.resume(co4)
print("  continued:", rok2, final)

-- Test 6: runtime error (not via error())
print("\nTest 6: runtime error")
local co5 = coroutine.create(function()
    local x = nil
    return x + 1  -- attempt to add nil
end)
local ok5, err5 = coroutine.resume(co5)
print("  ok:", ok5)
print("  status:", coroutine.status(co5))

print("\n=== Error handling tests done ===")
