print("=== Logical Return Combo Regression ===")

local function choose(a, b, c)
    return a and b or c
end

local chooseTrue = choose(true, "B", "C")
assert(chooseTrue == "B", "true and 'B' or 'C' should return 'B'")

local chooseFalse = choose(false, "B", "C")
assert(chooseFalse == "C", "false and 'B' or 'C' should return 'C'")

local chooseNil = choose(nil, "B", "C")
assert(chooseNil == "C", "nil and 'B' or 'C' should return 'C'")

local chooseZero = choose(0, "B", "C")
assert(chooseZero == "B", "0 is truthy in Lua")

local chooseFallback = choose(true, false, "fallback")
assert(chooseFallback == "fallback", "false middle value should fall through to fallback")

local rhsCalls = 0
local function mark(value)
    rhsCalls = rhsCalls + 1
    return value
end

local result1 = true and mark("kept") or "fallback"
assert(result1 == "kept", "true branch should keep RHS value")
assert(rhsCalls == 1, "RHS should run once for true and mark(...)")

local result2 = false and mark("ignored") or "fallback"
assert(result2 == "fallback", "false branch should skip RHS and use fallback")
assert(rhsCalls == 1, "RHS should not run for false and mark(...)")

local result3 = nil and mark("ignored") or "fallback"
assert(result3 == "fallback", "nil branch should skip RHS and use fallback")
assert(rhsCalls == 1, "RHS should still not run for nil and mark(...)")

local result4 = 0 and mark("zero-is-truthy") or "fallback"
assert(result4 == "zero-is-truthy", "0 should evaluate the RHS in Lua")
assert(rhsCalls == 2, "RHS should run for truthy 0")

print("Logical return combo regression passed")
