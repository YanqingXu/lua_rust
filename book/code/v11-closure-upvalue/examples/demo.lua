function make_state(start)
    local current = start

    local function add(delta)
        current = current + delta
        return current
    end

    local function get()
        return current
    end

    return add, get
end

local add, get = make_state(10)
local first = add(2)
local second = add(3)

local captured = {}
for index = 1, 3 do
    captured[index] = function()
        return index
    end
end

return first * 10000 + get() * 1000
    + captured[1]() * 100 + captured[2]() * 10 + captured[3]()
