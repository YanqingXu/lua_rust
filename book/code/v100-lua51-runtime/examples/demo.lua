local values = { 100 }
table.sort(values)

local worker = coroutine.wrap(function()
    return tonumber(table.concat(values, ""))
end)

local result = worker()
print(result)
return result
