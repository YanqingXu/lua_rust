-- Test getfenv() function
print("=== Testing getfenv() ===")

-- Test 1: Get environment of print function (C function)
print("Test 1: Get environment of C function (print)")
print("Environment type:", type(getfenv(print)))

-- Test 2: Get environment of a Lua function
print("Test 2: Get environment of Lua function")
function testFunc()
    return "Hello from testFunc"
end

print("Environment type:", type(getfenv(testFunc)))

print("=== All getfenv() tests passed! ===")

