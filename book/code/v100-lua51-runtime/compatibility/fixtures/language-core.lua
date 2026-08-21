local function values()
    return 10, 20, 30
end

local sum = 0
for index = 1, 4 do
    sum = sum + index
end

local a, b, c = values()
print(sum, a + b + c, (false or "lua") .. 5.1)
