local function recurse(depth)
    if depth == 0 then
        return 7
    end

    local function capture()
        return depth
    end
    local value = recurse(depth - 1)
    return value + capture()
end

local result = recurse(3)
assert(result == 13)
print("automatic gc closure resume", result)
error("automatic gc closure resume complete")
