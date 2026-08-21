function factorial(value)
    if value <= 1 then
        return 1
    end
    return value * factorial(value - 1)
end

local calculator = {base = 2}

function calculator:add(value)
    return self.base + value
end

return calculator:add(factorial(5))
