-- Test to demonstrate VM SELF instruction limitation
-- This test shows that userdata method calls fail

print("=== VM SELF Instruction Limitation Test ===")
print()

-- Test 1: Open a file
print("Test 1: Opening file...")
local f = io.open("test.txt", "w")
if f then
    print("File opened successfully")
    print("Type of f:", type(f))
else
    print("ERROR: Failed to open file")
end
print()

print("=== Test Complete ===")

