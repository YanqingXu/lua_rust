-- Known v0.15 boundary: tail-call arguments live only in active varargs at GC.
-- Expected current result: nonzero exit with "stale GC handle".
local function consume(...)
    gc_collect()
    local value = ...
    return value.answer
end

local function forward()
    return consume({ answer = 42 })
end

return forward()
