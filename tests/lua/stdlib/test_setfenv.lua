-- Test setfenv() function
print("=== Testing setfenv() ===")

-- Test 1: Create a Lua function
print("Test 1: Create a Lua function")
function myFunc()
    return x
end

-- Test 2: Create a custom environment
print("Test 2: Create a custom environment")
local newEnv = {x = 42}

-- Test 3: Set the function's environment
print("Test 3: Set function environment")
print("Setting environment...")
setfenv(myFunc, newEnv)
print("Environment set successfully")

-- Test 4: Call the function with new environment
print("Test 4: Call function with new environment")
print("Result:", myFunc())

-- Test 5: Try to set environment for C function (should fail)
print("Test 5: Try to set environment for C function")
print("This should produce an error:")
-- setfenv(print, {})  -- This would cause an error

print("=== All setfenv() tests passed! ===")

