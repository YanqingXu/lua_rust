local values = { 9, 2, 5 }
table.sort(values)

local text = string.upper(string.sub("lua-byte", 1, 3))
local first = string.byte(text, 1)
local ok, square = pcall(function(value)
    return math.pow(value, 2)
end, values[2])

local result = ok and (first * 20 + square + #table.concat(values, ",")) or 0
if arg then
    print(result)
end
return result
