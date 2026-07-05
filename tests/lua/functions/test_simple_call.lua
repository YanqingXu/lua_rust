-- Test simple table method call
local t = {}
function t.foo(a, b)
    return a, b
end

local x, y = t.foo("hello", "world")
print(x, y)

