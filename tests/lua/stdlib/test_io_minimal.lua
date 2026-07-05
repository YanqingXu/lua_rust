-- Minimal I/O test - single return value
print("=== Test 1: Call io.open ===")
local f = io.open("test.txt", "w")
print("Type of f:", type(f))
if f then
    print("SUCCESS: io.open returned a file handle")
    f:close()
else
    print("FAILED: io.open returned nil")
end

