local fallback = { missing = "from metatable" }
local data = { value = 1 }

setmetatable(data, { __index = fallback })

data.value = data.value + 1

print(data.value)
print(data.missing)
