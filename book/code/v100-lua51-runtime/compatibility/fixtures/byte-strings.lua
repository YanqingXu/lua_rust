local bytes = string.char(0, 65, 255)
local first, second, third = string.byte(bytes, 1, 3)
print(#bytes, first, second, third, string.byte(string.sub(bytes, 2, 2)))
