-- Test I/O Library Functions
print("=== Testing I/O Library ===")
print()

-- Test 1: io.open and file:write
print("Test 1: io.open and file:write")
local f = io.open("test_output.txt", "w")
if f then
    print("File opened successfully")
    f:write("Hello, World!\n")
    f:write("Line 2\n")
    f:write("Number: ", 42, "\n")
    f:close()
    print("File written and closed")
else
    print("Failed to open file")
end
print()

-- Test 2: io.open and file:read
print("Test 2: io.open and file:read")
f = io.open("test_output.txt", "r")
if f then
    print("File opened for reading")
    local line1 = f:read("*l")
    print("Line 1:", line1)
    local line2 = f:read("*l")
    print("Line 2:", line2)
    f:close()
    print("File closed")
else
    print("Failed to open file")
end
print()

-- Test 3: file:seek
print("Test 3: file:seek")
f = io.open("test_output.txt", "r")
if f then
    local pos = f:seek("end")
    print("File size:", pos, "bytes")
    f:seek("set", 0)
    local content = f:read("*a")
    print("Full content:")
    print(content)
    f:close()
else
    print("Failed to open file")
end
print()

-- Test 4: io.tmpfile
print("Test 4: io.tmpfile")
local tmpf = io.tmpfile()
if tmpf then
    print("Temporary file created")
    tmpf:write("Temporary data\n")
    tmpf:seek("set", 0)
    local data = tmpf:read("*a")
    print("Temp file content:", data)
    tmpf:close()
    print("Temporary file closed")
else
    print("Failed to create temporary file")
end
print()

-- Test 5: file:flush
print("Test 5: file:flush")
f = io.open("test_flush.txt", "w")
if f then
    f:write("Data before flush")
    f:flush()
    print("File flushed successfully")
    f:close()
else
    print("Failed to open file")
end
print()

-- Test 6: file:setvbuf
print("Test 6: file:setvbuf")
f = io.open("test_buffer.txt", "w")
if f then
    f:setvbuf("full", 1024)
    print("Buffer mode set successfully")
    f:write("Buffered data\n")
    f:close()
else
    print("Failed to open file")
end
print()

-- Test 7: io.type
print("Test 7: io.type")
f = io.open("test_output.txt", "r")
if f then
    print("Type of open file:", io.type(f))
    f:close()
    print("Type of closed file:", io.type(f))
else
    print("Failed to open file")
end
print("Type of non-file:", io.type("not a file"))
print()

-- Test 8: io.popen (if supported)
print("Test 8: io.popen")
local ok, result = pcall(function()
    local p = io.popen("echo Hello from pipe", "r")
    if p then
        print("Pipe opened successfully")
        local output = p:read("*a")
        print("Pipe output:", output)
        p:close()
    else
        print("Failed to open pipe")
    end
end)
if not ok then
    print("io.popen error:", result)
end
print()

-- Test 9: io.lines (if supported)
print("Test 9: io.lines")
ok, result = pcall(function()
    local count = 0
    for line in io.lines("test_output.txt") do
        print("Line:", line)
        count = count + 1
    end
    print("io.lines count:", count)

    local fh = io.open("test_output.txt", "r")
    if fh then
        local viaMethod = 0
        for line in fh:lines() do
            print("Method line:", line)
            viaMethod = viaMethod + 1
        end
        print("file:lines count:", viaMethod)
        fh:close()
    end
end)
if not ok then
    print("io.lines error:", result)
end
print()

print("=== All I/O tests completed ===")
