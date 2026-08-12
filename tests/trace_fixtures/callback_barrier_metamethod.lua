local value = setmetatable({}, {
    __add = function()
        return 7
    end,
})

local result = value + 1
assert(result == 7)
