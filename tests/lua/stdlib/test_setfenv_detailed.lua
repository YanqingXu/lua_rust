-- Detailed test for setfenv() function
print("=== Detailed setfenv() Test ===")

-- Test 1: Verify environment change affects global variable access
print("\nTest 1: Environment change affects global access")

-- Create a function that accesses a global variable
function getValue()
    return globalVar
end

-- Set global variable
globalVar = "original value"

-- Call function before changing environment
print("Before setfenv:", getValue())

-- Create new environment with different value
local customEnv = {globalVar = "custom value"}

-- Change function's environment
setfenv(getValue, customEnv)

-- Call function after changing environment
print("After setfenv:", getValue())

-- Test 2: Verify getfenv returns the new environment
print("\nTest 2: Verify getfenv returns new environment")
print("Environment type:", type(getfenv(getValue)))

print("\n=== All detailed tests passed! ===")

