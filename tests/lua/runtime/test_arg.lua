-- Test script for arg table functionality
-- This script verifies that the arg table is correctly populated
-- with command-line arguments according to Lua 5.1.5 specification

print("=== Lua arg Table Test ===")
print()

-- Test 1: Check if arg table exists
print("Test 1: arg table existence")
if arg then
    print("  [PASS] arg table exists")
else
    print("  [FAIL] arg table does not exist")
    return
end
print()

-- Test 2: Check arg[0] (script name)
print("Test 2: arg[0] (script name)")
print("  arg[0] =", arg[0])
print()

-- Test 3: Check arg[1], arg[2], arg[3] (command-line arguments)
print("Test 3: Command-line arguments")
print("  arg[1] =", arg[1])
print("  arg[2] =", arg[2])
print("  arg[3] =", arg[3])
print()

-- Test 4: Iterate through all arguments
print("Test 4: Iterate all arguments")
local count = 0
for i = 0, 10 do
    if arg[i] ~= nil then
        print(string.format("  arg[%d] = %s", i, tostring(arg[i])))
        count = count + 1
    else
        break
    end
end
print("  Total arguments:", count)
print()

-- Test 5: Check table length
print("Test 5: Table length")
local len = 0
for k, v in pairs(arg) do
    if type(k) == "number" and k >= 0 then
        len = len + 1
    end
end
print("  Calculated length:", len)
print()

print("=== Test Complete ===")

