function triple(...)
    return ...
end

function sum(...)
    local total = 0
    for index, value in ipairs({...}) do
        total = total + value
    end
    return total
end

function countdown(n, total)
    if n == 0 then
        return total
    end
    return countdown(n - 1, total + 1)
end

local a, b, c = triple(10, 20, 30)
return sum(a, b, c) + countdown(1000, 0)
