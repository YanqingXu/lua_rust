local function identity(value)
    return value
end

local number = identity(7)
local negated = -number

local values = { 10, 20, 30 }
local length = #values

local truthy = identity("truthy")
local inverted = not truthy

assert(negated == -7)
assert(length == 3)
assert(inverted == false)

return negated, length, inverted
