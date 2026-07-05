print("=== I/O Core Regression ===")

local path = string.format(
    "test_iolib_core_%d_%d.txt",
    os.time(),
    math.floor(os.clock() * 1000)
)
local f = io.open(path, "w")
assert(f, "io.open should return file handle")
assert(io.type(f) == "file", "open handle type should be file")

local str = tostring(f)
assert(type(str) == "string", "tostring(file) should return string")

f:write("alpha\n")
f:write("beta\n")
f:flush()
f:close()
assert(io.type(f) == "closed file", "closed handle type should be closed file")

local rf = io.open(path, "r")
assert(rf, "reopen for reading should succeed")
assert(rf:read("*l") == "alpha", "first line should match")
local pos = rf:seek("cur", 0)
assert(type(pos) == "number", "seek should return numeric position")
assert(rf:read("*l") == "beta", "second line should match")
rf:close()

local tf = io.tmpfile()
assert(tf, "tmpfile should succeed")
tf:write("temp-data")
tf:seek("set", 0)
assert(tf:read("*a") == "temp-data", "tmpfile readback should match")
tf:close()

local collected = {}
for line in io.lines(path) do
    collected[#collected + 1] = line
end
assert(#collected == 2, "io.lines should iterate two lines")
assert(collected[1] == "alpha", "io.lines first line should match")
assert(collected[2] == "beta", "io.lines second line should match")

local fh = io.open(path, "r")
assert(fh, "open for file:lines should succeed")
local methodCollected = {}
for line in fh:lines() do
    methodCollected[#methodCollected + 1] = line
end
assert(#methodCollected == 2, "file:lines should iterate two lines")
assert(methodCollected[1] == "alpha", "file:lines first line should match")
assert(methodCollected[2] == "beta", "file:lines second line should match")
fh:close()

local defaultPath = string.format(
    "test_iolib_default_%d_%d.txt",
    os.time(),
    math.floor(os.clock() * 1000)
)

local out = io.output(defaultPath)
assert(io.type(out) == "file", "io.output(filename) should return file handle")
local writeRet = io.write("gamma", 321)
assert(io.type(writeRet) == "file", "io.write should return current output handle")
assert(io.close(), "io.close() should close current default output")

local inp = io.input(defaultPath)
assert(io.type(inp) == "file", "io.input(filename) should return file handle")
assert(io.read(5) == "gamma", "io.read(count) should read from default input")
assert(io.read("*n") == 321, "io.read('*n') should read numeric suffix")
inp:close()

print("I/O core regression passed")
