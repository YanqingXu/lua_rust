-- Simple I/O Library Test
print("=== Simple I/O Test ===")

-- Test 1: io.open and file:write
print("Test 1: io.open and file:write")
local f = io.open("test.txt", "w")
print("File handle type:", type(f))

if f then
    print("File opened")
    f:write("Hello")
    f:close()
    print("File closed")
end

print("=== Test complete ===")

